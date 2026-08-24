//! Operations: small, serialisable, invertible commands
//! (docs/03-DATA-MODEL.md §10). Applying an op yields its inverse; the journal
//! of (op, inverse) pairs is the undo/redo stack and the crash-recovery log.

use crate::anim::Animation;
use crate::model::{
    BlendMode, Document, Layer, LinearColour, MatteRef, ProjectItem, TransformProp,
};
use crate::time::{CompTime, Duration, FrameRate};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OpError {
    #[error("unknown item")]
    UnknownItem,
    #[error("unknown composition")]
    UnknownComp,
    #[error("unknown layer")]
    UnknownLayer,
    #[error("index out of range")]
    BadIndex,
    #[error("invalid span: out point must be after in point")]
    InvalidSpan,
    #[error("invalid parent: would form a cycle, self-parent, or unknown layer")]
    InvalidParent,
    #[error("the layer is locked")]
    LayerLocked,
    /// The camera's placement is derived from a solve link, so it is not the
    /// document's to edit (K-417). Convert to keyframes
    /// ([`crate::track::bake_solve_link`]) makes it ordinary again, and clearing
    /// the link with [`Op::SetCameraSolveLink`] is always allowed.
    #[error("the camera's transform is derived from its solve link")]
    CameraLinked,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op_type")]
pub enum Op {
    /// Insert a project item at an index in the Project panel order.
    AddItem {
        index: usize,
        item: Box<ProjectItem>,
    },
    RemoveItem {
        id: Uuid,
    },
    /// Point a footage item at a different file (docs/07 §3.3 relink, docs/10
    /// §2). Carries the whole `MediaRef` — path pair and fingerprint — so it
    /// is trivially invertible and a relink is one undo step.
    SetMediaRef {
        id: Uuid,
        media: Box<crate::model::MediaRef>,
    },
    RenameItem {
        id: Uuid,
        name: String,
    },
    /// Set a project item's colour tag (K-451): an index into the same label
    /// palette a layer's chip uses, 0 = untagged. Undoable like any other edit;
    /// it changes no pixel.
    SetItemLabel {
        id: Uuid,
        label: u8,
    },
    /// Insert a layer at a stack index (0 = top) of a comp.
    AddLayer {
        comp: Uuid,
        index: usize,
        layer: Box<Layer>,
    },
    RemoveLayer {
        comp: Uuid,
        layer: Uuid,
    },
    /// Move a layer to a new stack index (0 = top), keeping the layer itself.
    /// One undoable step; the inverse moves it back to its old index. The
    /// index is the target position in the list once the layer is lifted out
    /// (ordinary `Vec::insert` semantics), clamped into range.
    ReorderLayer {
        comp: Uuid,
        layer: Uuid,
        new_index: usize,
    },
    /// Set a layer's span on the comp timeline.
    SetLayerSpan {
        comp: Uuid,
        layer: Uuid,
        in_point: CompTime,
        out_point: CompTime,
        start_offset: CompTime,
    },
    RenameLayer {
        comp: Uuid,
        layer: Uuid,
        name: String,
    },
    /// Replace a layer's whole mask list (coarse + exactly invertible, like
    /// SetTransformProperty; per-vertex ops arrive with the pen tool).
    SetLayerMasks {
        comp: Uuid,
        layer: Uuid,
        masks: Vec<crate::mask::Mask>,
    },
    /// Replace a layer's whole paint stroke list (docs/03 §7.1, K-227).
    ///
    /// The whole list, exactly invertible, exactly as `SetLayerMasks` is: a
    /// stroke added, deleted, recoloured or renamed is one shape of edit and one
    /// undo step. A stroke is a gesture and gestures arrive whole.
    SetLayerPaint {
        comp: Uuid,
        layer: Uuid,
        strokes: Vec<crate::paint::PaintStroke>,
    },
    /// Replace a shape layer's whole contents (docs/03 §7.2, K-237).
    ///
    /// The whole list, exactly invertible, like `SetLayerMasks` and
    /// `SetLayerPaint`: vector art arrives and changes as a whole.
    SetShapeContents {
        comp: Uuid,
        layer: Uuid,
        contents: Vec<crate::shape::ShapeItem>,
    },
    /// Replace a layer's whole effect stack (docs/03 §8; coarse + exactly
    /// invertible like SetLayerMasks — add/remove/reorder/param edits all
    /// commit the new list).
    SetLayerEffects {
        comp: Uuid,
        layer: Uuid,
        effects: Vec<crate::model::EffectInstance>,
    },
    /// The fx switch: bypass a layer's whole effect stack (docs/08 §1.5).
    SetLayerFx {
        comp: Uuid,
        layer: Uuid,
        fx: bool,
    },
    SetLayerThreeD {
        comp: Uuid,
        layer: Uuid,
        three_d: bool,
    },
    /// Replace a Sequence layer's whole clip list (coarse + exactly
    /// invertible, like SetLayerMasks; cutting/moving produce a new list).
    SetSequenceClips {
        comp: Uuid,
        layer: Uuid,
        clips: Vec<crate::sequence::Clip>,
    },
    /// Mute or unmute a layer's audio (the audible switch).
    SetLayerAudible {
        comp: Uuid,
        layer: Uuid,
        audible: bool,
    },
    /// Show or hide a layer (the visible switch).
    SetLayerVisible {
        comp: Uuid,
        layer: Uuid,
        visible: bool,
    },
    /// Toggle a layer's solo / isolate switch (K-105).
    SetLayerSolo {
        comp: Uuid,
        layer: Uuid,
        solo: bool,
    },
    /// Toggle a layer's per-layer motion-blur switch (K-120).
    SetLayerMotionBlur {
        comp: Uuid,
        layer: Uuid,
        motion_blur: bool,
    },
    /// Toggle a layer's Accepts lights switch (K-361): whether the comp's
    /// Light layers shade it.
    SetLayerAcceptsLights {
        comp: Uuid,
        layer: Uuid,
        accepts_lights: bool,
    },
    /// Toggle a layer's shy switch (docs/07 §4.2): hidden from the Timeline's
    /// list while the comp's shy filter is on. Never changes what renders.
    SetLayerShy {
        comp: Uuid,
        layer: Uuid,
        shy: bool,
    },
    /// Toggle a layer's lock (TL2): a locked layer's bar, trims and order are
    /// held still in the timeline.
    SetLayerLocked {
        comp: Uuid,
        layer: Uuid,
        locked: bool,
    },
    /// Set a layer's label colour (TL2): an index into the theme's label
    /// palette, shown as the chip beside the layer number.
    SetLayerLabel {
        comp: Uuid,
        layer: Uuid,
        label: u8,
    },
    /// Set a composition's motion-blur shutter (K-120): the master enable plus
    /// the shutter angle/phase and sample count.
    SetCompMotionBlur {
        comp: Uuid,
        motion_blur: crate::model::MotionBlur,
    },
    /// Set a composition's background colour (docs/07 §2.2 item 10, K-357).
    ///
    /// A document edit, unlike the Viewer's transparency grid (K-352) which
    /// only decides whether the backdrop is *drawn*: this is what colour it is
    /// when it is, and it reaches the export. Scene-linear, exactly
    /// invertible.
    SetCompBackground {
        comp: Uuid,
        background: crate::model::LinearColour,
    },
    /// Toggle a Precomp layer's collapse-transformations switch (docs/06 §1.4).
    SetLayerCollapse {
        comp: Uuid,
        layer: Uuid,
        collapse: bool,
    },
    /// Replace a Text layer's document (exactly invertible).
    SetTextDocument {
        comp: Uuid,
        layer: Uuid,
        document: crate::model::TextDocument,
    },
    SetWorkArea {
        comp: Uuid,
        work_area: Option<(CompTime, CompTime)>,
    },
    /// Replace a composition's whole marker list (coarse-grained, trivially
    /// invertible — beat regeneration builds the new list and commits this).
    SetCompMarkers {
        comp: Uuid,
        markers: Vec<crate::markers::Marker>,
    },
    /// Replace a layer's own marker list (docs/03 §11) — coarse-grained and
    /// trivially invertible, exactly like [`Op::SetCompMarkers`].
    SetLayerMarkers {
        comp: Uuid,
        layer: Uuid,
        markers: Vec<crate::markers::Marker>,
    },
    SetLayerBlend {
        comp: Uuid,
        layer: Uuid,
        blend: BlendMode,
    },
    /// Point a layer at another layer as its matte (or clear it).
    SetLayerMatte {
        comp: Uuid,
        layer: Uuid,
        matte: Option<MatteRef>,
    },
    /// Point a layer at another layer as its transform parent (or clear it,
    /// with `None`). A self-parent or a parent that would form a cycle, or a
    /// parent not in the comp, is rejected (`OpError::InvalidParent`).
    SetLayerParent {
        comp: Uuid,
        layer: Uuid,
        parent: Option<Uuid>,
    },
    /// Replace one transform property's whole animation (static or keyframed).
    /// Coarse-grained on purpose: trivially invertible; per-keyframe ops
    /// arrive with the graph editor.
    SetTransformProperty {
        comp: Uuid,
        layer: Uuid,
        prop: TransformProp,
        animation: Animation,
    },
    /// Replace a Camera layer's zoom animation (same coarse-grained shape as
    /// SetTransformProperty, for the same invertibility reason).
    SetCameraZoom {
        comp: Uuid,
        layer: Uuid,
        animation: Animation,
    },
    /// Point a Camera layer's **solve link** at a tracked layer, or clear it
    /// with `None` (K-417, docs/03 §5.6).
    ///
    /// The one edit a linked camera always accepts — everything else about its
    /// placement is derived and refuses with [`OpError::CameraLinked`], and a
    /// link that could not be undone would be a trap rather than a link.
    /// Trivially invertible: the inverse names the previous link.
    SetCameraSolveLink {
        comp: Uuid,
        layer: Uuid,
        solve_link: Option<Uuid>,
    },
    /// Replace a layer's audio Volume animation (docs/09 §6; same
    /// coarse-grained shape as SetTransformProperty, for the same
    /// invertibility reason). Valid on any layer; only heard where the
    /// source has an audio stream.
    SetLayerVolume {
        comp: Uuid,
        layer: Uuid,
        animation: Animation,
    },
    /// Replace a layer's Retime property — local time → source time, in
    /// seconds (K-197). `None` removes it, which is "not retimed" rather than
    /// "retimed to exactly 1×": only the first skips the map. Same
    /// coarse-grained shape as SetTransformProperty, for the same
    /// invertibility reason.
    SetRetimeProperty {
        comp: Uuid,
        layer: Uuid,
        retime: Option<crate::anim::Property>,
    },
    /// Set how a layer's fractional source moments become pixels — nearest,
    /// blend or flow (docs/04-RETIMING.md §10).
    ///
    /// A render policy, independent of the retime map, which is why it is its
    /// own op rather than part of `SetRetimeProperty`: changing how in-betweens
    /// are made must not touch the map, and un-retimed layers have the setting
    /// too.
    SetLayerInterpolation {
        comp: Uuid,
        layer: Uuid,
        interpolation: crate::retime::Interpolation,
        /// Where the layer's Flow tuning sits while the policy is not Flow
        /// ([`crate::model::Layer::parked_flow`]). Part of this op rather than
        /// its own, so turning flow off and putting the tuning away is one
        /// undo step that restores both together.
        parked_flow: Option<Box<crate::retime::FlowParams>>,
    },
    /// Several ops as one undo step (e.g. "create Solids folder + solid +
    /// layer"). Applied in order; the inverse is the reversed inverses. If a
    /// member fails, the already-applied members are rolled back, so a batch
    /// is all-or-nothing.
    Batch {
        ops: Vec<Op>,
    },
    /// Replace a folder's ordered children (coarse-grained: trivially
    /// invertible, and every move is one of these on each affected folder).
    SetFolderChildren {
        folder: Uuid,
        children: Vec<Uuid>,
    },
    /// Point an auto-filing slot (Solids / Compositions) at a folder.
    SetAutoFolder {
        kind: AutoFolderKind,
        folder: Option<Uuid>,
    },
    /// Set (or clear) this project's own cache location — where *its* rendered
    /// frames are parked, overriding the application-wide choice (docs/06 §5.4).
    ///
    /// An op like any other, so it is undoable, journalled and saved with the
    /// project. It changes no pixel: the frames a cache holds are named by their
    /// content, and where they are kept is not part of that name, so moving the
    /// folder costs nothing already cached elsewhere.
    SetCacheLocation {
        location: Option<crate::model::CacheLocation>,
    },
    /// Set how hard the renderer works at the edges of transformed layers
    /// (K-274, docs/impl/anti-aliasing.md).
    ///
    /// An op like any other — undoable, journalled and saved with the project —
    /// because it is a property of the project rather than a preference: it
    /// changes what the comp looks like, in the preview and in the export
    /// alike, so it has to travel in the file and be undoable like any other
    /// change to the picture.
    SetAntiAliasing {
        anti_aliasing: crate::model::AntiAliasing,
    },
    /// Edit a composition's settings after creation (AE: Composition
    /// Settings). Layers keep their spans; a shorter duration simply clips
    /// what plays.
    SetCompSettings {
        comp: Uuid,
        name: String,
        width: u32,
        height: u32,
        frame_rate: FrameRate,
        duration: Duration,
        background: LinearColour,
    },
    /// Edit a SolidDef asset (colour/size/name); every layer using it updates.
    SetSolidDef {
        def: Uuid,
        name: String,
        colour: LinearColour,
        width: u32,
        height: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AutoFolderKind {
    Solids,
    Compositions,
}

/// Apply `op` to `doc`, returning the exact inverse operation.
/// The `(comp, layer)` a lock would protect this op from, or `None` when the
/// lock has nothing to say about it.
///
/// **Lock protects the work, not the housekeeping.** A locked layer refuses
/// every edit to what it *is* — its transform, effects, masks, paint, art,
/// text, clips, markers, blend, matte, parent, retime, volume, switches, its
/// span, its place in the stack and its existence. It still accepts the three
/// that are about the Timeline's own bookkeeping rather than the composition:
/// the lock itself (or it could never be undone), **shy** (a filter on the
/// list, which changes no pixel and no timing) and the **label** colour.
///
/// Ops that name no layer — a comp setting, a project item, a folder — are not
/// the lock's business and answer `None`.
#[must_use]
fn lock_guards(op: &Op) -> Option<(Uuid, Uuid)> {
    match op {
        // The three a locked layer still accepts.
        Op::SetLayerLocked { .. } | Op::SetLayerShy { .. } | Op::SetLayerLabel { .. } => None,

        Op::RemoveLayer { comp, layer }
        | Op::ReorderLayer { comp, layer, .. }
        | Op::SetLayerSpan { comp, layer, .. }
        | Op::RenameLayer { comp, layer, .. }
        | Op::SetLayerMasks { comp, layer, .. }
        | Op::SetLayerPaint { comp, layer, .. }
        | Op::SetShapeContents { comp, layer, .. }
        | Op::SetLayerEffects { comp, layer, .. }
        | Op::SetLayerFx { comp, layer, .. }
        | Op::SetLayerThreeD { comp, layer, .. }
        | Op::SetSequenceClips { comp, layer, .. }
        | Op::SetLayerAudible { comp, layer, .. }
        | Op::SetLayerVisible { comp, layer, .. }
        | Op::SetLayerSolo { comp, layer, .. }
        | Op::SetLayerMotionBlur { comp, layer, .. }
        | Op::SetLayerAcceptsLights { comp, layer, .. }
        | Op::SetLayerCollapse { comp, layer, .. }
        | Op::SetTextDocument { comp, layer, .. }
        | Op::SetLayerMarkers { comp, layer, .. }
        | Op::SetLayerBlend { comp, layer, .. }
        | Op::SetLayerMatte { comp, layer, .. }
        | Op::SetLayerParent { comp, layer, .. }
        | Op::SetTransformProperty { comp, layer, .. }
        | Op::SetCameraZoom { comp, layer, .. }
        | Op::SetCameraSolveLink { comp, layer, .. }
        | Op::SetLayerVolume { comp, layer, .. }
        | Op::SetRetimeProperty { comp, layer, .. }
        | Op::SetLayerInterpolation { comp, layer, .. } => Some((*comp, *layer)),

        // Everything else names no layer to lock: comp settings, project items,
        // folders, and `Batch`, whose members are each guarded on their own way
        // through `apply`.
        _ => None,
    }
}

/// Whether `layer` in `comp` is locked. An unknown comp or layer is not locked:
/// the op's own arm reports what is actually missing, which is a better error
/// than "locked".
#[must_use]
fn is_locked(doc: &Document, comp: Uuid, layer: Uuid) -> bool {
    doc.comp(comp)
        .and_then(|c| c.layers.iter().find(|l| l.id == layer))
        .is_some_and(|l| l.switches.locked)
}

/// The `(comp, layer)` a **solve link** would protect this op from, or `None`
/// when the link has nothing to say about it (K-417).
///
/// A linked camera's placement is derived per frame from the tracked layer, so
/// the two ops that would write it — the transform and the zoom — refuse. Every
/// other edit to the layer is untouched: its name, its span, its switches, its
/// label and its markers are still the user's, and so is the link itself.
#[must_use]
fn solve_link_guards(op: &Op) -> Option<(Uuid, Uuid)> {
    match op {
        Op::SetTransformProperty { comp, layer, .. } | Op::SetCameraZoom { comp, layer, .. } => {
            Some((*comp, *layer))
        }
        _ => None,
    }
}

/// Whether `layer` in `comp` is a Camera carrying a solve link. An unknown comp
/// or layer is not linked — the op's own arm reports what is actually missing,
/// which is the better error (the lock's rule, for the same reason).
#[must_use]
fn is_solve_linked(doc: &Document, comp: Uuid, layer: Uuid) -> bool {
    doc.comp(comp)
        .and_then(|c| c.layers.iter().find(|l| l.id == layer))
        .is_some_and(|l| {
            matches!(
                l.kind,
                crate::model::LayerKind::Camera {
                    solve_link: Some(_),
                    ..
                }
            )
        })
}

pub fn apply(doc: &mut Document, op: &Op) -> Result<Op, OpError> {
    // **The lock is enforced here, not in the interface** (K-291). The Timeline
    // already refused the *gestures* — the bar, the razor, rename, reorder,
    // delete — but a locked layer's transform, effect and volume rows went on
    // editing it, so the switch did not mean what it says. One guard covers
    // every op, every caller and every op yet to be written; a guard per row
    // would have to be remembered each time a row is added, and forgetting one
    // is exactly how this hole opened.
    //
    // A `Batch` is guarded by its members: each goes through here on its way
    // in, and a refusal rolls the whole batch back, so a batch is still all or
    // nothing.
    if let Some((comp, layer)) = lock_guards(op) {
        if is_locked(doc, comp, layer) {
            return Err(OpError::LayerLocked);
        }
    }
    // **A linked camera's placement is not the document's to edit** (K-417).
    // The same shape of guard as the lock above, and here for the same reason:
    // the panel already draws the rows read-only and wearing a badge, but a
    // rule enforced only in the interface is a rule an expression, a preset or
    // the next caller does not know about. Convert to keyframes
    // (`track::bake_solve_link`) clears the link inside its own batch before
    // it writes, which is what lets a bake through this.
    if let Some((comp, layer)) = solve_link_guards(op) {
        if is_solve_linked(doc, comp, layer) {
            return Err(OpError::CameraLinked);
        }
    }
    match op {
        Op::AddItem { index, item } => {
            if *index > doc.items.len() {
                return Err(OpError::BadIndex);
            }
            doc.items.insert(*index, (**item).clone());
            Ok(Op::RemoveItem { id: item.id() })
        }
        Op::RemoveItem { id } => {
            let index = doc
                .items
                .iter()
                .position(|i| i.id() == *id)
                .ok_or(OpError::UnknownItem)?;
            let item = doc.items.remove(index);
            Ok(Op::AddItem {
                index,
                item: Box::new(item),
            })
        }
        Op::SetMediaRef { id, media } => {
            let crate::model::ProjectItem::Footage(f) =
                doc.item_mut(*id).ok_or(OpError::UnknownItem)?
            else {
                return Err(OpError::UnknownItem);
            };
            let previous = std::mem::replace(&mut f.media, (**media).clone());
            Ok(Op::SetMediaRef {
                id: *id,
                media: Box::new(previous),
            })
        }
        Op::RenameItem { id, name } => {
            let item = doc.item_mut(*id).ok_or(OpError::UnknownItem)?;
            let previous = item.name().to_owned();
            item.set_name(name.clone());
            Ok(Op::RenameItem {
                id: *id,
                name: previous,
            })
        }
        Op::SetItemLabel { id, label } => {
            // The item has to exist: a tag on nothing would sit in the map for
            // ever, and the panel would have asked about a row that is not
            // there.
            if doc.item(*id).is_none() {
                return Err(OpError::UnknownItem);
            }
            let previous = doc.item_label(*id);
            // Untagged is the absence of an entry, not an entry saying zero —
            // so tagging an item and untagging it again leaves the document
            // exactly as it was found, and a project nobody has tagged still
            // saves with no line for it.
            if *label == 0 {
                doc.item_labels.remove(id);
            } else {
                doc.item_labels.insert(*id, *label);
            }
            Ok(Op::SetItemLabel {
                id: *id,
                label: previous,
            })
        }
        Op::AddLayer { comp, index, layer } => {
            let c = doc.comp_mut(*comp).ok_or(OpError::UnknownComp)?;
            if *index > c.layers.len() {
                return Err(OpError::BadIndex);
            }
            if layer.out_point <= layer.in_point {
                return Err(OpError::InvalidSpan);
            }
            c.layers.insert(*index, (**layer).clone());
            Ok(Op::RemoveLayer {
                comp: *comp,
                layer: layer.id,
            })
        }
        Op::RemoveLayer { comp, layer } => {
            let c = doc.comp_mut(*comp).ok_or(OpError::UnknownComp)?;
            let index = c
                .layers
                .iter()
                .position(|l| l.id == *layer)
                .ok_or(OpError::UnknownLayer)?;
            let removed = c.layers.remove(index);
            Ok(Op::AddLayer {
                comp: *comp,
                index,
                layer: Box::new(removed),
            })
        }
        Op::ReorderLayer {
            comp,
            layer,
            new_index,
        } => {
            let c = doc.comp_mut(*comp).ok_or(OpError::UnknownComp)?;
            let old = c
                .layers
                .iter()
                .position(|l| l.id == *layer)
                .ok_or(OpError::UnknownLayer)?;
            let lifted = c.layers.remove(old);
            let idx = (*new_index).min(c.layers.len());
            c.layers.insert(idx, lifted);
            Ok(Op::ReorderLayer {
                comp: *comp,
                layer: *layer,
                new_index: old,
            })
        }
        Op::SetLayerSpan {
            comp,
            layer,
            in_point,
            out_point,
            start_offset,
        } => {
            if out_point <= in_point {
                return Err(OpError::InvalidSpan);
            }
            let c = doc.comp_mut(*comp).ok_or(OpError::UnknownComp)?;
            let l = c
                .layers
                .iter_mut()
                .find(|l| l.id == *layer)
                .ok_or(OpError::UnknownLayer)?;
            let inverse = Op::SetLayerSpan {
                comp: *comp,
                layer: *layer,
                in_point: l.in_point,
                out_point: l.out_point,
                start_offset: l.start_offset,
            };
            l.in_point = *in_point;
            l.out_point = *out_point;
            l.start_offset = *start_offset;
            Ok(inverse)
        }
        Op::SetLayerMasks { comp, layer, masks } => {
            let c = doc.comp_mut(*comp).ok_or(OpError::UnknownComp)?;
            let l = c
                .layers
                .iter_mut()
                .find(|l| l.id == *layer)
                .ok_or(OpError::UnknownLayer)?;
            let previous = std::mem::replace(&mut l.masks, masks.clone());
            Ok(Op::SetLayerMasks {
                comp: *comp,
                layer: *layer,
                masks: previous,
            })
        }
        Op::SetLayerPaint {
            comp,
            layer,
            strokes,
        } => {
            let c = doc.comp_mut(*comp).ok_or(OpError::UnknownComp)?;
            let l = c
                .layers
                .iter_mut()
                .find(|l| l.id == *layer)
                .ok_or(OpError::UnknownLayer)?;
            let previous = std::mem::replace(&mut l.paint, strokes.clone());
            Ok(Op::SetLayerPaint {
                comp: *comp,
                layer: *layer,
                strokes: previous,
            })
        }
        Op::SetShapeContents {
            comp,
            layer,
            contents,
        } => {
            let c = doc.comp_mut(*comp).ok_or(OpError::UnknownComp)?;
            let l = c
                .layers
                .iter_mut()
                .find(|l| l.id == *layer)
                .ok_or(OpError::UnknownLayer)?;
            let crate::model::LayerKind::Shape {
                contents: existing, ..
            } = &mut l.kind
            else {
                return Err(OpError::UnknownLayer);
            };
            let previous = std::mem::replace(existing, contents.clone());
            Ok(Op::SetShapeContents {
                comp: *comp,
                layer: *layer,
                contents: previous,
            })
        }
        Op::SetLayerEffects {
            comp,
            layer,
            effects,
        } => {
            let c = doc.comp_mut(*comp).ok_or(OpError::UnknownComp)?;
            let l = c
                .layers
                .iter_mut()
                .find(|l| l.id == *layer)
                .ok_or(OpError::UnknownLayer)?;
            let previous = std::mem::replace(&mut l.effects, effects.clone());
            Ok(Op::SetLayerEffects {
                comp: *comp,
                layer: *layer,
                effects: previous,
            })
        }
        Op::SetLayerFx { comp, layer, fx } => {
            let c = doc.comp_mut(*comp).ok_or(OpError::UnknownComp)?;
            let l = c
                .layers
                .iter_mut()
                .find(|l| l.id == *layer)
                .ok_or(OpError::UnknownLayer)?;
            let previous = std::mem::replace(&mut l.switches.fx, *fx);
            Ok(Op::SetLayerFx {
                comp: *comp,
                layer: *layer,
                fx: previous,
            })
        }
        Op::SetSequenceClips { comp, layer, clips } => {
            let c = doc.comp_mut(*comp).ok_or(OpError::UnknownComp)?;
            let l = c
                .layers
                .iter_mut()
                .find(|l| l.id == *layer)
                .ok_or(OpError::UnknownLayer)?;
            let crate::model::LayerKind::Sequence { clips: slot } = &mut l.kind else {
                return Err(OpError::UnknownLayer);
            };
            let previous = std::mem::replace(slot, clips.clone());
            Ok(Op::SetSequenceClips {
                comp: *comp,
                layer: *layer,
                clips: previous,
            })
        }
        Op::SetLayerThreeD {
            comp,
            layer,
            three_d,
        } => {
            let c = doc.comp_mut(*comp).ok_or(OpError::UnknownComp)?;
            let l = c
                .layers
                .iter_mut()
                .find(|l| l.id == *layer)
                .ok_or(OpError::UnknownLayer)?;
            let previous = std::mem::replace(&mut l.switches.three_d, *three_d);
            Ok(Op::SetLayerThreeD {
                comp: *comp,
                layer: *layer,
                three_d: previous,
            })
        }
        Op::SetLayerCollapse {
            comp,
            layer,
            collapse,
        } => {
            let c = doc.comp_mut(*comp).ok_or(OpError::UnknownComp)?;
            let l = c
                .layers
                .iter_mut()
                .find(|l| l.id == *layer)
                .ok_or(OpError::UnknownLayer)?;
            let previous = std::mem::replace(&mut l.switches.collapse, *collapse);
            Ok(Op::SetLayerCollapse {
                comp: *comp,
                layer: *layer,
                collapse: previous,
            })
        }
        Op::SetLayerAudible {
            comp,
            layer,
            audible,
        } => {
            let c = doc.comp_mut(*comp).ok_or(OpError::UnknownComp)?;
            let l = c
                .layers
                .iter_mut()
                .find(|l| l.id == *layer)
                .ok_or(OpError::UnknownLayer)?;
            let previous = std::mem::replace(&mut l.switches.audible, *audible);
            Ok(Op::SetLayerAudible {
                comp: *comp,
                layer: *layer,
                audible: previous,
            })
        }
        Op::SetLayerVisible {
            comp,
            layer,
            visible,
        } => {
            let c = doc.comp_mut(*comp).ok_or(OpError::UnknownComp)?;
            let l = c
                .layers
                .iter_mut()
                .find(|l| l.id == *layer)
                .ok_or(OpError::UnknownLayer)?;
            let previous = std::mem::replace(&mut l.switches.visible, *visible);
            Ok(Op::SetLayerVisible {
                comp: *comp,
                layer: *layer,
                visible: previous,
            })
        }
        Op::SetLayerSolo { comp, layer, solo } => {
            let c = doc.comp_mut(*comp).ok_or(OpError::UnknownComp)?;
            let l = c
                .layers
                .iter_mut()
                .find(|l| l.id == *layer)
                .ok_or(OpError::UnknownLayer)?;
            let previous = std::mem::replace(&mut l.switches.solo, *solo);
            Ok(Op::SetLayerSolo {
                comp: *comp,
                layer: *layer,
                solo: previous,
            })
        }
        Op::SetLayerShy { comp, layer, shy } => {
            let c = doc.comp_mut(*comp).ok_or(OpError::UnknownComp)?;
            let l = c
                .layers
                .iter_mut()
                .find(|l| l.id == *layer)
                .ok_or(OpError::UnknownLayer)?;
            let previous = std::mem::replace(&mut l.switches.shy, *shy);
            Ok(Op::SetLayerShy {
                comp: *comp,
                layer: *layer,
                shy: previous,
            })
        }
        Op::SetLayerLocked {
            comp,
            layer,
            locked,
        } => {
            let c = doc.comp_mut(*comp).ok_or(OpError::UnknownComp)?;
            let l = c
                .layers
                .iter_mut()
                .find(|l| l.id == *layer)
                .ok_or(OpError::UnknownLayer)?;
            let previous = std::mem::replace(&mut l.switches.locked, *locked);
            Ok(Op::SetLayerLocked {
                comp: *comp,
                layer: *layer,
                locked: previous,
            })
        }
        Op::SetLayerLabel { comp, layer, label } => {
            let c = doc.comp_mut(*comp).ok_or(OpError::UnknownComp)?;
            let l = c
                .layers
                .iter_mut()
                .find(|l| l.id == *layer)
                .ok_or(OpError::UnknownLayer)?;
            let previous = std::mem::replace(&mut l.label, *label);
            Ok(Op::SetLayerLabel {
                comp: *comp,
                layer: *layer,
                label: previous,
            })
        }
        Op::SetLayerMotionBlur {
            comp,
            layer,
            motion_blur,
        } => {
            let c = doc.comp_mut(*comp).ok_or(OpError::UnknownComp)?;
            let l = c
                .layers
                .iter_mut()
                .find(|l| l.id == *layer)
                .ok_or(OpError::UnknownLayer)?;
            let previous = std::mem::replace(&mut l.switches.motion_blur, *motion_blur);
            Ok(Op::SetLayerMotionBlur {
                comp: *comp,
                layer: *layer,
                motion_blur: previous,
            })
        }
        Op::SetLayerAcceptsLights {
            comp,
            layer,
            accepts_lights,
        } => {
            let c = doc.comp_mut(*comp).ok_or(OpError::UnknownComp)?;
            let l = c
                .layers
                .iter_mut()
                .find(|l| l.id == *layer)
                .ok_or(OpError::UnknownLayer)?;
            let previous = std::mem::replace(&mut l.switches.accepts_lights, *accepts_lights);
            Ok(Op::SetLayerAcceptsLights {
                comp: *comp,
                layer: *layer,
                accepts_lights: previous,
            })
        }
        Op::SetCompMotionBlur { comp, motion_blur } => {
            let c = doc.comp_mut(*comp).ok_or(OpError::UnknownComp)?;
            let previous = std::mem::replace(&mut c.motion_blur, *motion_blur);
            Ok(Op::SetCompMotionBlur {
                comp: *comp,
                motion_blur: previous,
            })
        }
        Op::SetCompBackground { comp, background } => {
            let c = doc.comp_mut(*comp).ok_or(OpError::UnknownComp)?;
            let previous = std::mem::replace(&mut c.background, *background);
            Ok(Op::SetCompBackground {
                comp: *comp,
                background: previous,
            })
        }
        Op::SetTextDocument {
            comp,
            layer,
            document,
        } => {
            let c = doc.comp_mut(*comp).ok_or(OpError::UnknownComp)?;
            let l = c
                .layers
                .iter_mut()
                .find(|l| l.id == *layer)
                .ok_or(OpError::UnknownLayer)?;
            let crate::model::LayerKind::Text { document: current } = &mut l.kind else {
                return Err(OpError::UnknownLayer);
            };
            let previous = std::mem::replace(current, document.clone());
            Ok(Op::SetTextDocument {
                comp: *comp,
                layer: *layer,
                document: previous,
            })
        }
        Op::SetWorkArea { comp, work_area } => {
            if let Some((a, b)) = work_area {
                if b <= a {
                    return Err(OpError::InvalidSpan);
                }
            }
            let c = doc.comp_mut(*comp).ok_or(OpError::UnknownComp)?;
            // **Clamped to the composition, here rather than at the caller.** A
            // work area outside the comp is not a thing the document can mean:
            // there are no frames there to work on, and the frame numbers it
            // implies go negative — which, cast unsigned for the cache fill,
            // became an enormous first frame and took the render worker down
            // with a `min > max`. Clamping in the op bounds every caller at once
            // (a drag, a keyboard nudge, an imported project) and makes the drag
            // simply stop at the edge, since the panel draws what the document
            // says. A span that survives the clamp with nothing left is refused
            // rather than stored empty.
            let end = crate::time::CompTime(c.duration.0);
            let work_area = match work_area {
                None => None,
                Some((a, b)) => {
                    let a = (*a).clamp(crate::time::CompTime::ZERO, end);
                    let b = (*b).clamp(crate::time::CompTime::ZERO, end);
                    if b <= a {
                        return Err(OpError::InvalidSpan);
                    }
                    Some((a, b))
                }
            };
            let previous = std::mem::replace(&mut c.work_area, work_area);
            Ok(Op::SetWorkArea {
                comp: *comp,
                work_area: previous,
            })
        }
        Op::SetCompMarkers { comp, markers } => {
            let c = doc.comp_mut(*comp).ok_or(OpError::UnknownComp)?;
            let previous = std::mem::replace(&mut c.markers, markers.clone());
            Ok(Op::SetCompMarkers {
                comp: *comp,
                markers: previous,
            })
        }
        Op::SetLayerMarkers {
            comp,
            layer,
            markers,
        } => {
            let c = doc.comp_mut(*comp).ok_or(OpError::UnknownComp)?;
            let l = c
                .layers
                .iter_mut()
                .find(|l| l.id == *layer)
                .ok_or(OpError::UnknownLayer)?;
            let previous = std::mem::replace(&mut l.markers, markers.clone());
            Ok(Op::SetLayerMarkers {
                comp: *comp,
                layer: *layer,
                markers: previous,
            })
        }
        Op::SetLayerBlend { comp, layer, blend } => {
            let c = doc.comp_mut(*comp).ok_or(OpError::UnknownComp)?;
            let l = c
                .layers
                .iter_mut()
                .find(|l| l.id == *layer)
                .ok_or(OpError::UnknownLayer)?;
            let previous = std::mem::replace(&mut l.blend, *blend);
            Ok(Op::SetLayerBlend {
                comp: *comp,
                layer: *layer,
                blend: previous,
            })
        }
        Op::SetLayerMatte { comp, layer, matte } => {
            let c = doc.comp_mut(*comp).ok_or(OpError::UnknownComp)?;
            let l = c
                .layers
                .iter_mut()
                .find(|l| l.id == *layer)
                .ok_or(OpError::UnknownLayer)?;
            let previous = std::mem::replace(&mut l.matte, *matte);
            Ok(Op::SetLayerMatte {
                comp: *comp,
                layer: *layer,
                matte: previous,
            })
        }
        Op::SetLayerParent {
            comp,
            layer,
            parent,
        } => {
            let c = doc.comp_mut(*comp).ok_or(OpError::UnknownComp)?;
            // Validate against the current comp before mutating: the target
            // layer must exist, and a Some(parent) must be a different, real
            // layer that does not already descend from `layer` (no cycle).
            if !c.layers.iter().any(|l| l.id == *layer) {
                return Err(OpError::UnknownLayer);
            }
            if let Some(p) = parent {
                if !c.layers.iter().any(|l| l.id == *p)
                    || crate::model::parenting_would_cycle(c, *layer, *p)
                {
                    return Err(OpError::InvalidParent);
                }
            }
            let l = c
                .layers
                .iter_mut()
                .find(|l| l.id == *layer)
                .ok_or(OpError::UnknownLayer)?;
            let previous = std::mem::replace(&mut l.parent, *parent);
            Ok(Op::SetLayerParent {
                comp: *comp,
                layer: *layer,
                parent: previous,
            })
        }
        Op::SetTransformProperty {
            comp,
            layer,
            prop,
            animation,
        } => {
            let c = doc.comp_mut(*comp).ok_or(OpError::UnknownComp)?;
            let l = c
                .layers
                .iter_mut()
                .find(|l| l.id == *layer)
                .ok_or(OpError::UnknownLayer)?;
            let slot = l.transform.get_mut(*prop);
            let previous = std::mem::replace(&mut slot.animation, animation.clone());
            Ok(Op::SetTransformProperty {
                comp: *comp,
                layer: *layer,
                prop: *prop,
                animation: previous,
            })
        }
        Op::SetCameraZoom {
            comp,
            layer,
            animation,
        } => {
            let c = doc.comp_mut(*comp).ok_or(OpError::UnknownComp)?;
            let l = c
                .layers
                .iter_mut()
                .find(|l| l.id == *layer)
                .ok_or(OpError::UnknownLayer)?;
            let crate::model::LayerKind::Camera { zoom, .. } = &mut l.kind else {
                return Err(OpError::UnknownLayer);
            };
            let previous = std::mem::replace(&mut zoom.animation, animation.clone());
            Ok(Op::SetCameraZoom {
                comp: *comp,
                layer: *layer,
                animation: previous,
            })
        }
        Op::SetCameraSolveLink {
            comp,
            layer,
            solve_link,
        } => {
            let c = doc.comp_mut(*comp).ok_or(OpError::UnknownComp)?;
            let l = c
                .layers
                .iter_mut()
                .find(|l| l.id == *layer)
                .ok_or(OpError::UnknownLayer)?;
            let crate::model::LayerKind::Camera {
                solve_link: slot, ..
            } = &mut l.kind
            else {
                return Err(OpError::UnknownLayer);
            };
            let previous = std::mem::replace(slot, *solve_link);
            Ok(Op::SetCameraSolveLink {
                comp: *comp,
                layer: *layer,
                solve_link: previous,
            })
        }
        Op::SetLayerVolume {
            comp,
            layer,
            animation,
        } => {
            let c = doc.comp_mut(*comp).ok_or(OpError::UnknownComp)?;
            let l = c
                .layers
                .iter_mut()
                .find(|l| l.id == *layer)
                .ok_or(OpError::UnknownLayer)?;
            let previous = std::mem::replace(&mut l.volume_db.animation, animation.clone());
            Ok(Op::SetLayerVolume {
                comp: *comp,
                layer: *layer,
                animation: previous,
            })
        }
        Op::SetRetimeProperty {
            comp,
            layer,
            retime,
        } => {
            let c = doc.comp_mut(*comp).ok_or(OpError::UnknownComp)?;
            let l = c
                .layers
                .iter_mut()
                .find(|l| l.id == *layer)
                .ok_or(OpError::UnknownLayer)?;
            let previous = std::mem::replace(&mut l.retime, retime.clone());
            Ok(Op::SetRetimeProperty {
                comp: *comp,
                layer: *layer,
                retime: previous,
            })
        }
        Op::SetLayerInterpolation {
            comp,
            layer,
            interpolation,
            parked_flow,
        } => {
            let c = doc.comp_mut(*comp).ok_or(OpError::UnknownComp)?;
            let l = c
                .layers
                .iter_mut()
                .find(|l| l.id == *layer)
                .ok_or(OpError::UnknownLayer)?;
            let previous = std::mem::replace(&mut l.interpolation, interpolation.clone());
            let previous_parked = std::mem::replace(&mut l.parked_flow, parked_flow.clone());
            Ok(Op::SetLayerInterpolation {
                comp: *comp,
                layer: *layer,
                interpolation: previous,
                parked_flow: previous_parked,
            })
        }
        Op::Batch { ops } => {
            let mut inverses = Vec::with_capacity(ops.len());
            for member in ops {
                match apply(doc, member) {
                    Ok(inv) => inverses.push(inv),
                    Err(e) => {
                        // Roll back what applied; rollback of a just-applied
                        // inverse cannot fail, but stay panic-free regardless.
                        for inv in inverses.iter().rev() {
                            let _ = apply(doc, inv);
                        }
                        return Err(e);
                    }
                }
            }
            inverses.reverse();
            Ok(Op::Batch { ops: inverses })
        }
        Op::SetFolderChildren { folder, children } => {
            let f = match doc.item_mut(*folder) {
                Some(ProjectItem::Folder(f)) => f,
                _ => return Err(OpError::UnknownItem),
            };
            let previous = std::mem::replace(&mut f.children, children.clone());
            Ok(Op::SetFolderChildren {
                folder: *folder,
                children: previous,
            })
        }
        Op::SetCacheLocation { location } => {
            let previous = std::mem::replace(&mut doc.cache_location, location.clone());
            Ok(Op::SetCacheLocation { location: previous })
        }
        Op::SetAntiAliasing { anti_aliasing } => {
            let previous = std::mem::replace(&mut doc.anti_aliasing, *anti_aliasing);
            Ok(Op::SetAntiAliasing {
                anti_aliasing: previous,
            })
        }
        Op::SetAutoFolder { kind, folder } => {
            let slot = match kind {
                AutoFolderKind::Solids => &mut doc.auto_folders.solids,
                AutoFolderKind::Compositions => &mut doc.auto_folders.compositions,
            };
            let previous = std::mem::replace(slot, *folder);
            Ok(Op::SetAutoFolder {
                kind: *kind,
                folder: previous,
            })
        }
        Op::SetCompSettings {
            comp,
            name,
            width,
            height,
            frame_rate,
            duration,
            background,
        } => {
            let c = doc.comp_mut(*comp).ok_or(OpError::UnknownComp)?;
            let inverse = Op::SetCompSettings {
                comp: *comp,
                name: std::mem::replace(&mut c.name, name.clone()),
                width: std::mem::replace(&mut c.width, *width),
                height: std::mem::replace(&mut c.height, *height),
                frame_rate: std::mem::replace(&mut c.frame_rate, *frame_rate),
                duration: std::mem::replace(&mut c.duration, *duration),
                background: std::mem::replace(&mut c.background, *background),
            };
            Ok(inverse)
        }
        Op::SetSolidDef {
            def,
            name,
            colour,
            width,
            height,
        } => {
            let s = match doc.item_mut(*def) {
                Some(ProjectItem::Solid(s)) => s,
                _ => return Err(OpError::UnknownItem),
            };
            let inverse = Op::SetSolidDef {
                def: *def,
                name: std::mem::replace(&mut s.name, name.clone()),
                colour: std::mem::replace(&mut s.colour, *colour),
                width: std::mem::replace(&mut s.width, *width),
                height: std::mem::replace(&mut s.height, *height),
            };
            Ok(inverse)
        }
        Op::RenameLayer { comp, layer, name } => {
            let c = doc.comp_mut(*comp).ok_or(OpError::UnknownComp)?;
            let l = c
                .layers
                .iter_mut()
                .find(|l| l.id == *layer)
                .ok_or(OpError::UnknownLayer)?;
            let previous = std::mem::replace(&mut l.name, name.clone());
            Ok(Op::RenameLayer {
                comp: *comp,
                layer: *layer,
                name: previous,
            })
        }
    }
}

/// The ops that make a folder — the Project panel's bottom-bar **Folder**
/// button (K-451, docs/07 §3.1) — plus the new folder's id.
///
/// # In plain terms
///
/// Making a folder is two decisions and one op: what it is called, where it
/// goes, and then the ordinary [`Op::AddItem`] every project item is born from.
/// The decisions are here, beside the ops, so that a folder made from the
/// button, from a menu and from a test all come out the same — the engine
/// decides, and the interface only asks (docs/17-BRIDGE-CONTRACT.md).
///
/// A blank name takes the next unused "Folder N". `parent` files the new folder
/// inside an existing one; `None` — and a parent that no longer exists — leaves
/// it at the panel root, which is where imported footage lands too.
///
/// **Nothing is committed here.** The caller commits the list as one
/// [`Op::Batch`], and that is what makes the whole thing one undo step: the
/// folder and its filing arrive and leave together.
#[must_use]
pub fn new_folder_ops(doc: &Document, name: &str, parent: Option<Uuid>) -> (Uuid, Vec<Op>) {
    let id = Uuid::now_v7();
    let name = if name.trim().is_empty() {
        next_folder_name(doc)
    } else {
        name.to_owned()
    };

    let mut ops = vec![Op::AddItem {
        index: doc.items.len(),
        item: Box::new(ProjectItem::Folder(crate::model::Folder {
            id,
            name,
            children: Vec::new(),
            extra: serde_json::Map::new(),
        })),
    }];
    if let Some(folder) = parent.and_then(|p| doc.folder(p)) {
        let mut children = folder.children.clone();
        children.push(id);
        ops.push(Op::SetFolderChildren {
            folder: folder.id,
            children,
        });
    }
    (id, ops)
}

/// The next unused "Folder N" — the default name [`new_folder_ops`] gives.
///
/// Counts up past the names already taken rather than off the number of
/// folders: a project holding "Folder 1" and "Renders" must not offer
/// "Folder 2" only to collide the moment "Folder 1" is renamed away.
#[must_use]
fn next_folder_name(doc: &Document) -> String {
    let taken: std::collections::HashSet<&str> = doc
        .items
        .iter()
        .filter(|i| matches!(i, ProjectItem::Folder(_)))
        .map(ProjectItem::name)
        .collect();
    (1..)
        .map(|n| format!("Folder {n}"))
        .find(|candidate| !taken.contains(candidate.as_str()))
        // `1..` is unbounded and `taken` is finite, so this always answers; the
        // fallback is here because nothing in an engine crate may panic
        // (docs/14 §4).
        .unwrap_or_else(|| "Folder".to_owned())
}

/// The four playhead-relative span edits behind the `[` / `]` / `Alt+[` / `Alt+]`
/// keys (docs/07-UI-SPEC.md §4.7), the After Effects convention:
/// - `MoveIn`/`MoveOut` **move** the whole layer so its in/out point lands on the
///   playhead, keeping its duration and the source content shown at that edge
///   (in, out and `start_offset` all shift by the same delta);
/// - `TrimIn`/`TrimOut` **trim** one edge to the playhead, leaving `start_offset`
///   (so the same source frames still play at the same comp times).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpanEdit {
    MoveIn,
    MoveOut,
    TrimIn,
    TrimOut,
}

/// Compute the new `(in_point, out_point, start_offset)` for a [`SpanEdit`] at
/// `playhead`, all in comp time. Returns `None` when the edit would be
/// degenerate (a trim that would leave `out_point <= in_point`) or overflow.
#[must_use]
pub fn edit_layer_span(
    in_point: CompTime,
    out_point: CompTime,
    start_offset: CompTime,
    playhead: CompTime,
    edit: SpanEdit,
) -> Option<(CompTime, CompTime, CompTime)> {
    match edit {
        SpanEdit::MoveIn => {
            let d = playhead.delta(in_point).ok()?; // playhead − in
            Some((
                playhead,
                out_point.add_dur(d).ok()?,
                start_offset.add_dur(d).ok()?,
            ))
        }
        SpanEdit::MoveOut => {
            let d = playhead.delta(out_point).ok()?; // playhead − out
            Some((
                in_point.add_dur(d).ok()?,
                playhead,
                start_offset.add_dur(d).ok()?,
            ))
        }
        SpanEdit::TrimIn => (playhead < out_point).then_some((playhead, out_point, start_offset)),
        SpanEdit::TrimOut => (playhead > in_point).then_some((in_point, playhead, start_offset)),
    }
}

/// The span a layer takes when its Retime is switched off (K-212).
///
/// **In plain terms:** while a layer is retimed it can be any length, because
/// it chooses which source moment each of its own frames shows. Switch that off
/// and the layer plays at source rate again, so it has to be re-hung on the
/// source. The frame that was already showing at the in point is the anchor:
/// the layer keeps its in point and shows that same frame there, then carries
/// on at source rate until either the source runs out or the layer's existing
/// out point arrives — whichever comes first. It never grows: a layer trimmed
/// short stays short.
///
/// `anchor` is the source moment showing at `in_point`, read through the map
/// that is about to be removed. `source_length` is the source's own length, or
/// `None` when it has none (or could not be read) — then only the anchor is
/// honoured and the out point is left alone.
///
/// Returns `(in_point, out_point, start_offset)`, or `None` when the arithmetic
/// would overflow or the result would not be a real span — in which case the
/// caller leaves the span exactly as it found it.
#[must_use]
pub fn unretimed_span(
    in_point: CompTime,
    out_point: CompTime,
    anchor: crate::time::SourceTime,
    source_length: Option<Duration>,
) -> Option<(CompTime, CompTime, CompTime)> {
    // Layer time zero goes wherever it must for `anchor` to show at the in
    // point: an un-retimed layer reads its source at its own clock, so the
    // offset *is* the difference between the two.
    let start_offset = in_point.sub_dur(Duration(anchor.0)).ok()?;
    let out = match source_length {
        Some(len) => {
            let available = len.0.checked_sub(anchor.0).ok()?;
            // Anchored at or past the end of the source there is nothing to
            // measure from; the layer holds its last frame and keeps its span.
            if available.is_negative() || available.is_zero() {
                out_point
            } else {
                out_point.min(in_point.add_dur(Duration(available)).ok()?)
            }
        }
        None => out_point,
    };
    (out > in_point).then_some((in_point, out, start_offset))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod unretime_tests {
    use super::*;
    use crate::time::{Rational, SourceTime};

    fn ct(secs: i64) -> CompTime {
        CompTime(Rational::new(secs, 1).unwrap())
    }
    fn st(secs: i64) -> SourceTime {
        SourceTime(Rational::new(secs, 1).unwrap())
    }
    fn dur(secs: i64) -> Duration {
        Duration(Rational::new(secs, 1).unwrap())
    }

    /// The simple case: the layer was showing the source's first frame, so it
    /// simply plays from there until the source runs out.
    #[test]
    fn anchored_on_the_first_frame_runs_to_the_source_end() {
        // In at comp 10, showing source 0, and a 5-second source: the layer
        // ends at comp 15 however long the retimed version was.
        let (i, o, off) = unretimed_span(ct(10), ct(100), st(0), Some(dur(5))).unwrap();
        assert_eq!((i, o, off), (ct(10), ct(15), ct(10)));
    }

    /// Anchored part-way in, only what is left of the source is available.
    #[test]
    fn anchored_mid_source_runs_out_sooner() {
        let (i, o, off) = unretimed_span(ct(10), ct(100), st(2), Some(dur(5))).unwrap();
        assert_eq!(i, ct(10));
        assert_eq!(o, ct(13), "three seconds of source were left");
        assert_eq!(off, ct(8), "source zero sits two seconds before the in");
    }

    /// It never grows: a layer already shorter than the source keeps its length.
    #[test]
    fn a_trimmed_layer_keeps_its_length() {
        let (_, o, _) = unretimed_span(ct(10), ct(12), st(0), Some(dur(5))).unwrap();
        assert_eq!(o, ct(12));
    }

    /// No readable length — missing media, or a kind with no source of its own
    /// — re-anchors and leaves the out point alone rather than guessing.
    #[test]
    fn no_source_length_leaves_the_out_point_alone() {
        let (i, o, off) = unretimed_span(ct(10), ct(100), st(2), None).unwrap();
        assert_eq!((i, o, off), (ct(10), ct(100), ct(8)));
    }

    /// Anchored at or past the end of the source, there is nothing left to
    /// measure: the span survives intact rather than collapsing to nothing.
    #[test]
    fn an_anchor_past_the_source_end_keeps_the_span() {
        let (_, o, off) = unretimed_span(ct(10), ct(20), st(5), Some(dur(5))).unwrap();
        assert_eq!(o, ct(20));
        assert_eq!(off, ct(5));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod span_edit_tests {
    use super::*;
    use crate::time::Rational;

    fn ct(secs: i64) -> CompTime {
        CompTime(Rational::new(secs, 1).unwrap())
    }

    #[test]
    fn move_in_shifts_the_whole_layer_to_the_playhead() {
        // Layer visible [2,5), source-0 at comp 1; move its in point to 10.
        let (i, o, off) = edit_layer_span(ct(2), ct(5), ct(1), ct(10), SpanEdit::MoveIn).unwrap();
        // In lands on the playhead; duration (3s) and the source-at-in are kept.
        assert_eq!((i, o, off), (ct(10), ct(13), ct(9)));
    }

    #[test]
    fn move_out_puts_the_out_point_on_the_playhead() {
        let (i, o, off) = edit_layer_span(ct(2), ct(5), ct(1), ct(10), SpanEdit::MoveOut).unwrap();
        // Out lands on 10; duration 3s kept, so in = 7, offset shifts by +5.
        assert_eq!((i, o, off), (ct(7), ct(10), ct(6)));
    }

    #[test]
    fn trim_moves_one_edge_and_keeps_the_offset() {
        let (i, o, off) = edit_layer_span(ct(2), ct(5), ct(1), ct(3), SpanEdit::TrimIn).unwrap();
        assert_eq!((i, o, off), (ct(3), ct(5), ct(1)));
        let (i, o, off) = edit_layer_span(ct(2), ct(5), ct(1), ct(4), SpanEdit::TrimOut).unwrap();
        assert_eq!((i, o, off), (ct(2), ct(4), ct(1)));
    }

    #[test]
    fn a_degenerate_trim_is_rejected() {
        // Trimming the in point to or past the out point would invert the span.
        assert!(edit_layer_span(ct(2), ct(5), ct(1), ct(5), SpanEdit::TrimIn).is_none());
        assert!(edit_layer_span(ct(2), ct(5), ct(1), ct(6), SpanEdit::TrimIn).is_none());
        assert!(edit_layer_span(ct(2), ct(5), ct(1), ct(2), SpanEdit::TrimOut).is_none());
        // A move never inverts (duration is preserved), even past comp 0.
        let (i, o, _) = edit_layer_span(ct(2), ct(5), ct(1), ct(0), SpanEdit::MoveOut).unwrap();
        assert!(i < o);
    }
}

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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Op {
    /// Insert a project item at an index in the Project panel order.
    AddItem {
        index: usize,
        item: Box<ProjectItem>,
    },
    RemoveItem {
        id: Uuid,
    },
    RenameItem {
        id: Uuid,
        name: String,
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
    /// Replace a layer's audio Volume animation (docs/09 §6; same
    /// coarse-grained shape as SetTransformProperty, for the same
    /// invertibility reason). Valid on any layer; only heard where the
    /// source has an audio stream.
    SetLayerVolume {
        comp: Uuid,
        layer: Uuid,
        animation: Animation,
    },
    /// Replace a Footage layer's Retime map (None = play at source rate).
    SetLayerRetime {
        comp: Uuid,
        layer: Uuid,
        retime: Option<crate::retime::Retime>,
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
pub fn apply(doc: &mut Document, op: &Op) -> Result<Op, OpError> {
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
        Op::RenameItem { id, name } => {
            let item = doc.item_mut(*id).ok_or(OpError::UnknownItem)?;
            let previous = item.name().to_owned();
            item.set_name(name.clone());
            Ok(Op::RenameItem {
                id: *id,
                name: previous,
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
        Op::SetCompMotionBlur { comp, motion_blur } => {
            let c = doc.comp_mut(*comp).ok_or(OpError::UnknownComp)?;
            let previous = std::mem::replace(&mut c.motion_blur, *motion_blur);
            Ok(Op::SetCompMotionBlur {
                comp: *comp,
                motion_blur: previous,
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
            let previous = std::mem::replace(&mut c.work_area, *work_area);
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
            let crate::model::LayerKind::Camera { zoom } = &mut l.kind else {
                return Err(OpError::UnknownLayer);
            };
            let previous = std::mem::replace(&mut zoom.animation, animation.clone());
            Ok(Op::SetCameraZoom {
                comp: *comp,
                layer: *layer,
                animation: previous,
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
        Op::SetLayerRetime {
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
            let crate::model::LayerKind::Footage { retime: slot, .. } = &mut l.kind else {
                return Err(OpError::UnknownLayer);
            };
            let previous = std::mem::replace(slot, retime.clone());
            Ok(Op::SetLayerRetime {
                comp: *comp,
                layer: *layer,
                retime: previous,
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

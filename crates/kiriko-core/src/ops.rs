//! Operations: small, serialisable, invertible commands
//! (docs/03-DATA-MODEL.md §10). Applying an op yields its inverse; the journal
//! of (op, inverse) pairs is the undo/redo stack and the crash-recovery log.

use crate::anim::Animation;
use crate::model::{BlendMode, Document, Layer, MatteRef, ProjectItem, TransformProp};
use crate::time::CompTime;
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
    /// Replace one transform property's whole animation (static or keyframed).
    /// Coarse-grained on purpose: trivially invertible; per-keyframe ops
    /// arrive with the graph editor.
    SetTransformProperty {
        comp: Uuid,
        layer: Uuid,
        prop: TransformProp,
        animation: Animation,
    },
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

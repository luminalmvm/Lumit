//! Which ops a layer's lock protects it from.

use super::*;

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
pub(super) fn lock_guards(op: &Op) -> Option<(Uuid, Uuid)> {
    match op {
        // The three a locked layer still accepts.
        Op::SetLayerLocked { .. } | Op::SetLayerShy { .. } | Op::SetLayerLabel { .. } => None,

        Op::RemoveLayer { comp, layer }
        | Op::ReorderLayer { comp, layer, .. }
        | Op::SetLayerSpan { comp, layer, .. }
        | Op::RenameLayer { comp, layer, .. }
        | Op::SetLayerMasks { comp, layer, .. }
        | Op::SetLayerPaint { comp, layer, .. }
        | Op::SetLayerPuppet { comp, layer, .. }
        | Op::SetShapeContents { comp, layer, .. }
        | Op::SetLayerEffects { comp, layer, .. }
        | Op::SetLayerStyles { comp, layer, .. }
        | Op::SetLayerGraphInputs { comp, layer, .. }
        | Op::SetLayerGradeVersions { comp, layer, .. }
        | Op::SetLayerGraph { comp, layer, .. }
        | Op::SetLayerFx { comp, layer, .. }
        | Op::SetLayerThreeD { comp, layer, .. }
        | Op::SetSequenceClips { comp, layer, .. }
        | Op::SetLayerKind { comp, layer, .. }
        | Op::SetLayerOrient { comp, layer, .. }
        | Op::SetLayerAdjustment { comp, layer, .. }
        | Op::SetLayerPitchFollowsSpeed { comp, layer, .. }
        | Op::SetLayerAudible { comp, layer, .. }
        | Op::SetLayerVisible { comp, layer, .. }
        | Op::SetLayerSolo { comp, layer, .. }
        | Op::SetLayerGuide { comp, layer, .. }
        | Op::SetLayerMotionBlur { comp, layer, .. }
        | Op::SetLayerAcceptsLights { comp, layer, .. }
        | Op::SetLayerCollapse { comp, layer, .. }
        | Op::SetTextDocument { comp, layer, .. }
        | Op::EditCaptions { comp, layer, .. }
        | Op::SetLayerMarkers { comp, layer, .. }
        | Op::SetLayerBlend { comp, layer, .. }
        | Op::SetLayerMatte { comp, layer, .. }
        | Op::SetLayerParent { comp, layer, .. }
        | Op::SetTransformProperty { comp, layer, .. }
        | Op::SetTransformExpressionSlot { comp, layer, .. }
        | Op::SetTransformAxisMode { comp, layer, .. }
        | Op::SetRotationOrder { comp, layer, .. }
        | Op::SetCameraZoom { comp, layer, .. }
        | Op::SetCameraSettings { comp, layer, .. }
        | Op::SetLightSettings { comp, layer, .. }
        | Op::SetMaterialSettings { comp, layer, .. }
        | Op::SetLayerGeometry { comp, layer, .. }
        | Op::SetCameraSolveLink { comp, layer, .. }
        | Op::SetLayerVolume { comp, layer, .. }
        | Op::SetLayerPan { comp, layer, .. }
        | Op::SetLayerRoutes { comp, layer, .. }
        | Op::SetLayerAudioStreams { comp, layer, .. }
        | Op::SetLayerAudioGain { comp, layer, .. }
        | Op::SetLayerAutomation { comp, layer, .. }
        | Op::SetLayerSoloSafe { comp, layer, .. }
        | Op::SetLayerDuck { comp, layer, .. }
        | Op::SetRetimeProperty { comp, layer, .. }
        | Op::SetLayerInterpolation { comp, layer, .. } => Some((*comp, *layer)),

        // Everything else names no layer to lock: comp settings, project items,
        // folders, and `Batch`, whose members are each guarded on their own way
        // through `apply`.
        _ => None,
    }
}

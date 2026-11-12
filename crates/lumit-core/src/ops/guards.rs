//! Which ops need a composition that holds layers.

use super::*;

/// The composition an op edits **the layers of**, or `None` when the op has
/// nothing to do with a layer stack.
///
/// **A node graph has no layers, and the engine says so** (docs/impl/
/// node-graph-comp.md §1.1). One guard beside the lock's covers every op that
/// adds, removes, reorders or edits a layer or a group of them, so a present or
/// future layer op cannot forget to check, and a `Batch` is covered through its
/// members.
///
/// The comp-wide settings are deliberately not here: size, rate, duration, the
/// work area, a trim, a crop, markers, motion blur and the master fader are all
/// legal on a node graph, because a node graph is a composition.
#[must_use]
pub(super) fn comp_guards(op: &Op) -> Option<Uuid> {
    match op {
        Op::AddLayer { comp, .. }
        | Op::RemoveLayer { comp, .. }
        | Op::ReorderLayer { comp, .. }
        | Op::SetLayerSpan { comp, .. }
        | Op::RenameLayer { comp, .. }
        | Op::SetLayerMasks { comp, .. }
        | Op::SetLayerPaint { comp, .. }
        | Op::SetLayerPuppet { comp, .. }
        | Op::SetShapeContents { comp, .. }
        | Op::SetLayerEffects { comp, .. }
        | Op::SetLayerStyles { comp, .. }
        | Op::SetLayerGraphInputs { comp, .. }
        | Op::SetLayerGradeVersions { comp, .. }
        | Op::SetLayerGraph { comp, .. }
        | Op::SetLayerFx { comp, .. }
        | Op::SetLayerThreeD { comp, .. }
        | Op::SetSequenceClips { comp, .. }
        | Op::SetLayerKind { comp, .. }
        | Op::SetLayerOrient { comp, .. }
        | Op::SetLayerAdjustment { comp, .. }
        | Op::SetLayerPitchFollowsSpeed { comp, .. }
        | Op::SetLayerAudible { comp, .. }
        | Op::SetLayerVisible { comp, .. }
        | Op::SetLayerSolo { comp, .. }
        | Op::SetLayerMotionBlur { comp, .. }
        | Op::SetLayerAcceptsLights { comp, .. }
        | Op::SetLayerShy { comp, .. }
        | Op::SetLayerGuide { comp, .. }
        | Op::SetLayerLocked { comp, .. }
        | Op::SetLayerLabel { comp, .. }
        | Op::SetLayerCollapse { comp, .. }
        | Op::SetTextDocument { comp, .. }
        | Op::EditCaptions { comp, .. }
        | Op::SetLayerMarkers { comp, .. }
        | Op::SetLayerBlend { comp, .. }
        | Op::SetLayerMatte { comp, .. }
        | Op::SetLayerParent { comp, .. }
        | Op::SetTransformProperty { comp, .. }
        | Op::SetTransformExpressionSlot { comp, .. }
        | Op::SetTransformAxisMode { comp, .. }
        | Op::SetRotationOrder { comp, .. }
        | Op::SetCameraZoom { comp, .. }
        | Op::SetCameraSolveLink { comp, .. }
        | Op::SetLayerVolume { comp, .. }
        | Op::SetLayerPan { comp, .. }
        | Op::SetLayerRoutes { comp, .. }
        | Op::SetLayerAudioStreams { comp, .. }
        | Op::SetLayerAudioGain { comp, .. }
        | Op::SetLayerAutomation { comp, .. }
        | Op::SetLayerSoloSafe { comp, .. }
        | Op::SetLayerDuck { comp, .. }
        | Op::SetRetimeProperty { comp, .. }
        | Op::SetLayerInterpolation { comp, .. }
        | Op::GroupLayers { comp, .. }
        | Op::UngroupLayers { comp, .. }
        | Op::SetGroupName { comp, .. }
        | Op::SetGroupLabel { comp, .. }
        | Op::SetGroupEffects { comp, .. } => Some(*comp),
        _ => None,
    }
}

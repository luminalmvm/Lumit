//! What an op touches, which says who has to hear about it.

use uuid::Uuid;

/// The scope of one op: which composition it touches, which layer within it,
/// and whether it changed the project item list.
///
/// Matching the enum rather than sniffing a JSON blob for `comp`/`layer` string
/// fields is the whole point: every project-level op used to fall through with
/// nothing set, so the Project panel had no way to tell "an item was added" from
/// "someone nudged an opacity keyframe" and rebuilt on both.
///
/// Structural layer ops (add / remove / reorder) report the comp but not the
/// layer: what changed is the comp's layer list, not one layer's contents.
pub(crate) fn op_scope(op: &lumit_core::ops::Op) -> (Option<Uuid>, Option<Uuid>, bool) {
    use lumit_core::ops::Op;
    match op {
        // The project item list itself.
        Op::AddItem { .. }
        | Op::RemoveItem { .. }
        | Op::RenameItem { .. }
        // A colour tag tints the item's row icon and feeds the panel's filter
        // chips, so the panel has to hear about it.
        | Op::SetItemLabel { .. }
        // A comment is organisation too. No panel draws it yet.
        | Op::SetItemComment { .. }
        // The marks are organisation like a colour tag: no picture changes.
        | Op::SetFootageTrim { .. }
        | Op::SetMediaRef { .. }
        // A run of stills read at another rate is another picture at every
        // frame that reads it, and the panel's rate column says so, so the
        // item scope carries it exactly as a relink does.
        | Op::SetSequenceRate { .. }
        // A proxy is a second media reference on a footage item, and
        // all three of these change what the item's row says about itself —
        // whether it has a stand-in, whether it is being used, and the
        // project-wide switch that governs every row at once. All of them also
        // rename every frame that reads the item, so the panel and the cache
        // bar both have to hear.
        | Op::SetItemProxy { .. }
        | Op::SetItemUseProxy { .. }
        | Op::SetUseProxies { .. }
        // Read by the File menu when it opens, and it is a document change
        // like the proxy switch above.
        | Op::SetAutoPack { .. }
        | Op::SetFolderChildren { .. }
        | Op::SetAutoFolder { .. }
        // Where this project parks its frames. No panel draws it — Settings
        // reads it directly — but it is a document change like any other, so it
        // belongs in the item scope rather than in a silent default.
        | Op::SetCacheLocation { .. }
        // How hard the renderer works at the edges. No panel draws it
        // either — Settings reads it directly — but it is a document change,
        // and one that renames every frame of every comp, so it must be
        // reported rather than fall through silently.
        | Op::SetAntiAliasing { .. }
        | Op::SetColourDepth { .. }
        | Op::SetGraphRoom { .. }
        // Which OCIO config the project's colour names come from, and what one
        // footage item arrives as. Both rename frames — the config
        // every frame of every comp, the assignment every frame reading that
        // item — and the item's row will name its space, so both report.
        | Op::SetColourConfig { .. }
        | Op::SetColourWorkingSpace { .. }
        | Op::SetFootageColourSpace { .. }
        | Op::SetFootageInterpretation { .. }
        // The project's colour shelf. No panel draws it — the picker
        // reads it when it opens — but it is a document change like the cache
        // location above, so it reports rather than passing silently.
        | Op::SetProjectSwatches { .. }
        // A solid def is a project item, and its name shows in the panel.
        | Op::SetSolidDef { .. } => (None, None, true),

        // Comp settings carry the comp's name, so the panel row changes too.
        // Trimming and cropping are comp settings plus the layers they move,
        // so they report the widest scope the settings do.
        Op::SetCompSettings { comp, .. }
        | Op::SetCompStart { comp, .. }
        | Op::TrimCompToWorkArea { comp }
        | Op::CropCompToRegion { comp, .. } => (Some(*comp), None, true),

        // The comp, but no one layer.
        Op::AddLayer { comp, .. }
        | Op::RemoveLayer { comp, .. }
        | Op::ReorderLayer { comp, .. }
        | Op::SetCompMotionBlur { comp, .. }
        | Op::SetCompLighting { comp, .. }
        | Op::SetCompBackground { comp, .. }
        | Op::SetMasterVolume { comp, .. }
        // The layer Timeline's Sound mix row stands or falls on the mark, so
        // the whole outline rebuilds and no one layer changed.
        | Op::SetSoundMix { comp, .. }
        | Op::SetWorkArea { comp, .. }
        | Op::SetCompMarkers { comp, .. }
        // The beat band redraws from it, and it travels with the markers.
        | Op::SetBeatGrid { comp, .. }
        // A layer group is a band over a run of rows, so the outline
        // rebuilds and no one layer's contents changed. The comp is the honest
        // scope for all four: making one, taking it away, and renaming or
        // recolouring the header row it draws.
        | Op::GroupLayers { comp, .. }
        | Op::UngroupLayers { comp, .. }
        | Op::SetGroupName { comp, .. }
        | Op::SetGroupLabel { comp, .. }
        // A header's effect stack scopes to the whole run of members it acts
        // on, so the comp is the honest scope, as for the adjustment
        // switch below.
        | Op::SetGroupEffects { comp, .. }
        // A node graph's whole graph, beside the group header's stack: a
        // comp-wide change with no layer in it, so the comp model refreshes and
        // no layer builder does.
        | Op::SetCompGraph { comp, .. }
        // A layer that becomes an adjustment starts acting on everything
        // beneath it, and one that stops leaves those layers alone again — so
        // the comp is the honest scope, not the one row that changed.
        | Op::SetLayerKind { comp, .. }
        | Op::SetLayerOrient { comp, .. }
        | Op::SetLayerAdjustment { comp, .. } => (Some(*comp), None, false),

        // One layer's own contents.
        Op::SetLayerSpan { comp, layer, .. }
        | Op::SetLayerMarkers { comp, layer, .. }
        | Op::RenameLayer { comp, layer, .. }
        | Op::SetLayerMasks { comp, layer, .. }
        | Op::SetLayerPaint { comp, layer, .. }
        | Op::SetLayerPuppet { comp, layer, .. }
        | Op::SetShapeContents { comp, layer, .. }
        | Op::SetLayerEffects { comp, layer, .. }
        | Op::SetLayerStyles { comp, layer, .. }
        | Op::SetLayerGraphInputs { comp, layer, .. }
        | Op::SetLayerGraph { comp, layer, .. }
        | Op::SetLayerFx { comp, layer, .. }
        | Op::SetLayerThreeD { comp, layer, .. }
        | Op::SetSequenceClips { comp, layer, .. }
        | Op::SetLayerPitchFollowsSpeed { comp, layer, .. }
        | Op::SetLayerAudible { comp, layer, .. }
        | Op::SetLayerVisible { comp, layer, .. }
        | Op::SetLayerSolo { comp, layer, .. }
        | Op::SetLayerMotionBlur { comp, layer, .. }
        | Op::SetLayerAcceptsLights { comp, layer, .. }
        | Op::SetLayerShy { comp, layer, .. }
        | Op::SetLayerGuide { comp, layer, .. }
        | Op::SetLayerLocked { comp, layer, .. }
        | Op::SetLayerLabel { comp, layer, .. }
        | Op::SetLayerCollapse { comp, layer, .. }
        | Op::SetTextDocument { comp, layer, .. }
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
        | Op::SetLayerInterpolation { comp, layer, .. }
        | Op::SetRetimeProperty { comp, layer, .. } => (Some(*comp), Some(*layer), false),

        // The Notes panel rereads on this. No item or layer changed.
        Op::SetNotes { .. } => (None, None, false),

        // A batch is as broad as its members: the item flag is the union, and
        // the reference scope widens to "no one subtree" rather than picking a
        // member's comp and leaving the others unrefreshed.
        Op::Batch { ops } => (None, None, ops.iter().any(|o| op_scope(o).2)),
    }
}

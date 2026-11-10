//! Pictures of one thing on its own: the footage view and the layer view
//! (docs/impl/multi-viewer.md §3.6).

use super::*;

/// Draw one item on its own: the footage view and the layer view
/// (docs/impl/multi-viewer.md §3.6).
///
/// **No second render path.** A scratch composition is built around the item
/// on a clone of the document and composited by the ordinary walk, so the
/// picture arrives down the same zero-copy transport at the same quality, and
/// the frame names itself the way every other frame does. Nothing is
/// committed: the composition never reaches the document, the journal, the
/// undo stack or the Project panel.
#[frb(ignore)]
pub(super) fn render_item(
    req: RenderItemRequest,
    state: &mut WorkerState,
    stream: &mut WorkerResponseStream,
) -> Result<(), BridgeError> {
    let document = {
        let document = req.project.state()?;
        let document = document.read().map_err(|_| BridgeError::ReadFailed)?;
        document.store.snapshot()
    };
    // A nonsense rate reads as 25, which is a rate rather than a division by
    // zero: a view of a still needs one only to have a clock at all.
    let Ok(rate) = lumit_core::time::FrameRate::new(req.rate.0, req.rate.1)
        .or_else(|_| lumit_core::time::FrameRate::new(25, 1))
    else {
        return Ok(());
    };
    // The item's own length, as a time: frames over the rate, exactly.
    let Ok(duration) = lumit_core::time::Rational::from_i128(
        i128::from(req.frames.max(1)) * i128::from(req.rate.1.max(1)),
        i128::from(req.rate.0.max(1)),
    ) else {
        return Ok(());
    };
    let duration = lumit_core::time::Duration(duration);
    let Some(document) =
        crate::scratch::document_with(&document, req.of, req.effects, req.size, rate, duration)
    else {
        // The item has gone, or the layer has. The view shows its empty state
        // rather than a picture of something else.
        return Ok(());
    };
    state.set_view_id(req.view);
    look_at(state, req.view);
    // Not `last_shown`: the fill, the cache bar and the drags follow the
    // active composition view, and this item's scratch comp is none of those.
    let comp = req.of.comp_id();
    watched(state, stream, req.frame, |state, stream| {
        publish_frame(
            state,
            comp,
            req.frame,
            req.scale,
            &document,
            stream,
            BridgePlaybackMode::Adaptive,
            // A scratch composition is not the document, so its frames are not
            // banked against one: cacheable would file a picture under a
            // composition nothing else can ever ask for.
            false,
        );
    });
    Ok(())
}

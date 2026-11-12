//! The Scopes panel's traces: what one request is called in the queue, and,
//! on the desktop, the trace itself. Runs on the render worker's thread.

use flutter_rust_bridge::frb;

use super::RenderScopeRequest;
#[cfg(not(target_arch = "wasm32"))]
use super::{
    quality_for, viewed_kept, zero_copy_wants_bgra, WorkerResponse, WorkerResponseStream,
    WorkerState,
};
#[cfg(not(target_arch = "wasm32"))]
use crate::api::BridgeError;

impl RenderScopeRequest {
    /// Who is asking, as a number: the same composition, view, traces and
    /// colours are the same asker, whatever frame it wants. In the queue a
    /// newer request only replaces an older one with the same lane, so two
    /// Scopes panels, or a panel and the Levels row, never starve each other.
    #[frb(ignore)]
    pub(super) fn lane(&self) -> u64 {
        // FNV-1a, so the number is the same on every run.
        let bytes = self
            .comp
            .id
            .as_bytes()
            .iter()
            .copied()
            .chain(self.view.to_le_bytes())
            .chain(self.kinds.iter().flat_map(|k| k.to_le_bytes()))
            .chain(self.colours.iter().flatten().copied());
        bytes.fold(0xcbf2_9ce4_8422_2325, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
        })
    }
}

/// Trace `frame` and publish the result.
///
/// Always a CPU read-back even on a zero-copy build: the binning kernel needs
/// the pixels, and on those builds nothing ever brings them back. A failure
/// publishes nothing rather than taking the worker down — a scope that cannot
/// draw is a blank panel, not a lost picture.
#[frb(ignore)]
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn trace_scope(
    req: RenderScopeRequest,
    state: &mut WorkerState,
    stream: &mut WorkerResponseStream,
) -> Result<(), BridgeError> {
    let (document, revision) = {
        let document = state.project.state()?;
        let document = document.read().map_err(|_| BridgeError::ReadFailed)?;
        document.store.snapshot_and_revision()
    };
    // The picture that view shows: through its own 3D view, and the chip.
    // The kept copy, which playback has usually made already.
    let document = viewed_kept(
        &mut state.viewed,
        state.prefix,
        state.camera_views.get(&req.view).copied(),
        revision,
        document,
    );

    // Reuse the picture the Viewer already has, at whatever resolution it was
    // made at. Scopes read the *values* in a frame, so any size answers the
    // question — and compositing the composition a second time to ask it was
    // doubling the cost of every played frame with the panel open.
    // Only a frame that is still what this position *shows* will do:
    // an edit renames every frame it touches, and the entry the edit orphaned
    // keeps claiming the position it was made for. Asked at the quality each
    // candidate was made at, so a Half-resolution frame is judged by the Half
    // name and not by the Full one.
    // The card first: on a zero-copy build the frame the Viewer shows lives
    // there and nowhere else, so a frame the bar showed green was composited
    // a second time for every trace of it. Reading it back is a copy.
    let quality = quality_for(req.scale);
    let bgra = zero_copy_wants_bgra();
    let named = |(width, height, mut bytes): (u32, u32, Vec<u8>)| {
        if bgra {
            // The Scopes bin R, G and B by name.
            for px in bytes.chunks_exact_mut(4) {
                px.swap(0, 2);
            }
        }
        (width, height, bytes)
    };
    // The picture on screen before anything: while a value is dragged the
    // Viewer shows a preview the document has not been given, and a colourist
    // grades by the scope, so it must trace what the drag is doing. Only the
    // frame the Viewer was last given, in that view; playback presents its
    // frames another way and they are read off the card below.
    let shown = state
        .presented
        .as_ref()
        .filter(|_| state.playback.is_none())
        .filter(|(view, comp, frame, _)| {
            (*view, *comp, *frame) == (req.view, req.comp.id, req.frame)
        })
        .and_then(|(.., prepared)| state.renderer.read_back_prepared(prepared))
        .map(named);
    let on_card = match shown {
        Some(shown) => Some(shown),
        None => state
            .renderer
            .frame_key(&document, req.comp.id, req.frame, quality)
            .and_then(|key| state.renderer.read_back_frame_texture(key, bgra))
            .map(named),
    };
    let still_current = |key: u128, quality: lumit_render::plan::Quality| {
        state
            .renderer
            .frame_key_presynced(&document, req.comp.id, req.frame, quality)
            == Some(key)
    };
    let held =
        on_card.or_else(|| crate::framecache::best_frame(req.comp.id, req.frame, still_current));
    let (width, height, rgba) = match held {
        Some(held) => held,
        None => {
            // Nothing held for this frame — the zero-copy Viewer keeps no bytes,
            // so on that path the trace still has to make its own. Cached under
            // the frame's content name, so a second trace of the same frame is
            // free; an unnameable frame (footage still being probed) is traced
            // without banking anything.
            let key = state
                .renderer
                .frame_key(&document, req.comp.id, req.frame, quality);
            let provenance = lumit_render::headless::FrameProvenance {
                comp: req.comp.id,
                frame: req.frame,
                scale_q: lumit_render::headless::preview_scale_q(quality),
                quality,
            };
            let made = match key.and_then(crate::framecache::get) {
                Some(hit) => Some(hit),
                None => {
                    // A flare that fell back to the previous lens during the
                    // render made a picture the name taken before it
                    // no longer describes. Banked only when no flare stood
                    // anything in.
                    let subs_before = state.renderer.flare_substitutions();
                    let made = state
                        .renderer
                        .render_preview(
                            &document,
                            req.comp.id,
                            req.frame,
                            quality_for(req.scale),
                            req.scale,
                        )
                        .ok()
                        .map(|(rgba, width, height)| (width, height, rgba));
                    if let (Some(key), Some((w, h, px))) = (key, made.as_ref()) {
                        if state.renderer.flare_substitutions() == subs_before {
                            crate::framecache::put_rendered(key, provenance, *w, *h, px);
                        }
                    }
                    made
                }
            };
            let Some(made) = made else {
                note!("Scope render failed, dropping the trace");
                return Ok(());
            };
            made
        }
    };

    // Every kind asked for from the one picture, each its own reply.
    match state
        .renderer
        .render_scope(&rgba, width, height, &req.kinds, req.colours)
    {
        Ok(traces) => {
            for (&kind, rgba) in req.kinds.iter().zip(traces) {
                _ = stream.add(WorkerResponse::Scope(crate::api::state::BridgeScopeTrace {
                    kind,
                    rgba,
                }));
            }
        }
        Err(err) => note!("Scope trace failed: {err}"),
    }
    Ok(())
}

//! The Scopes, the dropper and playback's footage in the browser build.
//!
//! A copy off the card lands only once the page's event loop has run, which is
//! after the turn that asked for it. So here a trace or a dropper read starts
//! its copy and a later turn finishes it ([`finish`]). The desktop waits for
//! the copy in place, in the functions of the same names one level up.

use std::cell::RefCell;

use lumit_gpu::PendingReadback;
use uuid::Uuid;

use super::{
    cut_patch, quality_for, soloed, viewed_kept, viewed_through, zero_copy_wants_bgra, LayerSample,
    RenderScopeRequest, SamplePixelsRequest, WorkerResponse, WorkerResponseStream, WorkerState,
};
use crate::api::BridgeError;

/// What a copy is for, once it lands.
enum Then {
    /// The frame, to trace.
    Trace(RenderScopeRequest),
    /// The finished trace of this kind, to publish.
    Publish(u32),
    /// The frame, for the dropper, with the stamp a soloed layer's picture is
    /// kept under.
    Sample(
        SamplePixelsRequest,
        Option<(Uuid, u64, Uuid, u64, u32, bool)>,
    ),
}

impl Then {
    /// Which job this is: a trace of one kind, or the dropper. Only the newest
    /// copy of each is worth finishing.
    fn job(&self) -> Option<u32> {
        match self {
            Then::Trace(req) => Some(req.kind),
            Then::Publish(kind) => Some(*kind),
            Then::Sample(..) => None,
        }
    }
}

/// The name a rendered frame is banked under once it lands, as the desktop
/// banks it straight after the render.
type Bank = Option<(u128, lumit_render::headless::FrameProvenance)>;

thread_local! {
    static READS: RefCell<Vec<(PendingReadback, Then, Bank)>> = const { RefCell::new(Vec::new()) };
}

/// Wait for `read`, in place of an older one for the same job.
fn wait_for(read: PendingReadback, then: Then, bank: Bank) {
    READS.with(|reads| {
        let mut reads = reads.borrow_mut();
        reads.retain(|(_, older, _)| older.job() != then.job());
        reads.push((read, then, bank));
    });
}

/// Render `frame` of `document` and start reading it back, with the name to
/// bank it under: none for a frame that can't be named, or that drew a flare
/// or a read-back from an earlier frame.
fn start_render(
    state: &mut WorkerState,
    document: &std::sync::Arc<lumit_core::model::Document>,
    comp: Uuid,
    frame: u64,
    scale: f32,
    name: Option<u128>,
) -> Result<(PendingReadback, Bank), String> {
    let quality = quality_for(scale);
    let subs_before = state.renderer.flare_substitutions();
    let read = state
        .renderer
        .start_preview_scaled(document, comp, frame, quality, scale)?;
    let provenance = lumit_render::headless::FrameProvenance {
        comp,
        frame,
        scale_q: lumit_render::headless::preview_scale_q(quality),
        quality,
    };
    let bank = name
        .filter(|_| state.renderer.flare_substitutions() == subs_before)
        .map(|key| (key, provenance));
    Ok((read, bank))
}

/// Finish the copies that have landed, and make the picture again when an
/// effect's read-back came back with a new answer. Once a turn.
pub(super) fn finish(state: &mut WorkerState, stream: &mut WorkerResponseStream) {
    thread_local! {
        static SEEN: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    }
    let landed = lumit_gpu::device::reads_landed();
    if state.playback.is_none() && SEEN.with(|seen| seen.replace(landed)) != landed {
        super::republish_last_frame(state, stream);
    }
    let reads = READS.with(|reads| std::mem::take(&mut *reads.borrow_mut()));
    for (mut read, then, bank) in reads {
        let (width, height) = read.size();
        match state.renderer.poll_read(&mut read) {
            // Still on its way, unless a newer one for the job started meanwhile.
            None => READS.with(|reads| {
                let mut reads = reads.borrow_mut();
                if !reads.iter().any(|(_, newer, _)| newer.job() == then.job()) {
                    reads.push((read, then, bank));
                }
            }),
            Some(Ok(rgba)) => {
                if let Some((key, provenance)) = bank {
                    crate::framecache::put_rendered(key, provenance, width, height, &rgba);
                }
                land(state, stream, then, &rgba, width, height);
            }
            Some(Err(err)) => note!("A read-back failed: {err}"),
        }
    }
    // Back soon while a copy is on its way.
    if READS.with(|reads| !reads.borrow().is_empty()) {
        crate::turns::pause(std::time::Duration::from_millis(4));
    }
}

/// Carry on with a picture that has arrived.
fn land(
    state: &mut WorkerState,
    stream: &mut WorkerResponseStream,
    then: Then,
    rgba: &[u8],
    width: u32,
    height: u32,
) {
    match then {
        Then::Trace(req) => {
            match state
                .renderer
                .start_scope(rgba, width, height, req.kind, req.colours)
            {
                Ok(read) => wait_for(read, Then::Publish(req.kind), None),
                Err(err) => note!("Scope trace failed: {err}"),
            }
        }
        Then::Publish(kind) => {
            _ = stream.add(WorkerResponse::Scope(crate::api::state::BridgeScopeTrace {
                kind,
                rgba: rgba.to_vec(),
            }));
        }
        Then::Sample(req, stamp) => {
            if let Some(stamp) = stamp {
                state.layer_sample = Some(LayerSample {
                    stamp,
                    width,
                    height,
                    rgba: rgba.to_vec(),
                });
            }
            publish_sample(stream, &req, rgba, width, height, false);
        }
    }
}

/// The dropper's window out of a picture, sent. Nothing when there is none.
fn publish_sample(
    stream: &mut WorkerResponseStream,
    req: &SamplePixelsRequest,
    rgba: &[u8],
    width: u32,
    height: u32,
    bgra: bool,
) {
    let (u, v) = (req.u.clamp(0.0, 1.0), req.v.clamp(0.0, 1.0));
    let Some(mut patch) = cut_patch(rgba, width, height, u, v, req.window) else {
        return;
    };
    if bgra {
        for px in patch.rgba.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
    }
    _ = stream.add(WorkerResponse::Sampled(
        crate::api::state::BridgeSampledPixels {
            window: patch.window,
            rgba: patch.rgba,
            width,
            height,
            x: patch.x,
            y: patch.y,
            frame: req.frame,
            view: req.view,
            layer_alone: req.layer.is_some(),
        },
    ));
}

/// The document as it stands, and its revision.
fn snapshot(
    state: &WorkerState,
) -> Result<(std::sync::Arc<lumit_core::model::Document>, u64), BridgeError> {
    let document = state.project.state()?;
    let document = document.read().map_err(|_| BridgeError::ReadFailed)?;
    Ok((document.store.snapshot(), document.store.revision()))
}

/// Trace `frame`: the picture the Viewer holds on the card, else the one kept
/// in memory, else a render of it, as on the desktop. The trace is published
/// when it lands.
pub(super) fn trace_scope(
    req: RenderScopeRequest,
    state: &mut WorkerState,
    stream: &mut WorkerResponseStream,
) -> Result<(), BridgeError> {
    let (document, revision) = snapshot(state)?;
    let document = viewed_kept(
        &mut state.viewed,
        state.prefix,
        state.camera_views.get(&req.view).copied(),
        revision,
        document,
    );
    let quality = quality_for(req.scale);
    let key = state
        .renderer
        .frame_key(&document, req.comp.id, req.frame, quality);
    if let Some(read) =
        key.and_then(|key| state.renderer.start_held_read(key, zero_copy_wants_bgra()))
    {
        wait_for(read, Then::Trace(req), None);
        return Ok(());
    }
    let still_current = |key: u128, quality: lumit_render::plan::Quality| {
        state
            .renderer
            .frame_key_presynced(&document, req.comp.id, req.frame, quality)
            == Some(key)
    };
    let held = crate::framecache::best_frame(req.comp.id, req.frame, still_current)
        .or_else(|| key.and_then(crate::framecache::get));
    if let Some((width, height, rgba)) = held {
        land(state, stream, Then::Trace(req), &rgba, width, height);
        return Ok(());
    }
    match start_render(state, &document, req.comp.id, req.frame, req.scale, key) {
        Ok((read, bank)) => wait_for(read, Then::Trace(req), bank),
        Err(err) => note!("Scope render failed, dropping the trace: {err}"),
    }
    Ok(())
}

/// Read the pixels under the dropper from the same places as on the desktop:
/// a soloed layer's kept picture or a render of it, else the frame held on the
/// card, the one kept in memory, or a render. The window is published when its
/// picture lands.
pub(super) fn sample_pixels(
    req: SamplePixelsRequest,
    state: &mut WorkerState,
    stream: &mut WorkerResponseStream,
) -> Result<(), BridgeError> {
    let (document, revision) = snapshot(state)?;
    let quality = quality_for(req.scale);

    if let Some(layer) = req.layer.as_ref().map(|layer| layer.layer_id) {
        let stamp = (
            req.comp.id,
            req.frame,
            layer,
            revision,
            req.view,
            req.part_ids,
        );
        if let Some(held) = state
            .layer_sample
            .as_ref()
            .filter(|held| held.stamp == stamp)
        {
            publish_sample(stream, &req, &held.rgba, held.width, held.height, false);
            return Ok(());
        }
        let Some(alone) = soloed(&document, layer, &req, state) else {
            return Ok(());
        };
        // Parts are read at full size with the Viewer's own view off, as in
        // the native read.
        let (quality, scale) = if req.part_ids {
            (quality_for(1.0), 1.0)
        } else {
            (quality, req.scale)
        };
        let view = state.renderer.display_view();
        if req.part_ids {
            state
                .renderer
                .set_display_view(lumit_gpu::DisplayParams::NEUTRAL);
        }
        let started =
            state
                .renderer
                .start_preview_scaled(&alone, req.comp.id, req.frame, quality, scale);
        state.renderer.set_display_view(view);
        match started {
            Ok(read) => wait_for(read, Then::Sample(req, Some(stamp)), None),
            Err(err) => note!("Dropper render failed: {err}"),
        }
        return Ok(());
    }

    let document = viewed_through(None, state.camera_views.get(&req.view).copied(), document);
    let name = state
        .renderer
        .frame_key(&document, req.comp.id, req.frame, quality);
    if let Some(read) =
        name.and_then(|key| state.renderer.start_held_read(key, zero_copy_wants_bgra()))
    {
        wait_for(read, Then::Sample(req, None), None);
        return Ok(());
    }
    let still_current = |key: u128, quality: lumit_render::plan::Quality| {
        state
            .renderer
            .frame_key_presynced(&document, req.comp.id, req.frame, quality)
            == Some(key)
    };
    let banked = crate::framecache::with_best_frame(
        req.comp.id,
        req.frame,
        still_current,
        |bytes, w, h, bgra| {
            publish_sample(stream, &req, bytes, w, h, bgra);
        },
    );
    if banked.is_some() {
        return Ok(());
    }
    if let Some((width, height, rgba)) = name.and_then(crate::framecache::get) {
        publish_sample(stream, &req, &rgba, width, height, false);
        return Ok(());
    }
    match start_render(state, &document, req.comp.id, req.frame, req.scale, name) {
        Ok((read, bank)) => wait_for(read, Then::Sample(req, None), bank),
        Err(err) => note!("Dropper render failed: {err}"),
    }
    Ok(())
}

/// Whether playback holds `frame` back for footage the page is still
/// decoding: it is asked for, and drawn once it lands or the clock reaches
/// it, whichever is first. A stand-in drawn early would be shown in its place.
/// A frame a tier holds needs no footage.
pub(super) fn footage_waits(
    state: &mut WorkerState,
    document: &std::sync::Arc<lumit_core::model::Document>,
    comp: Uuid,
    frame: u64,
    quality: lumit_render::plan::Quality,
) -> bool {
    let bgra = super::zero_copy_wants_bgra();
    let name = state.renderer.frame_key(document, comp, frame, quality);
    if name.is_some_and(|key| super::held_in_a_tier(&state.renderer, &state.disk, key, bgra)) {
        return false;
    }
    let wants = state
        .renderer
        .prefetch_wants(document, comp, frame, quality);
    // Every want is asked for, not just the first missing one.
    let landed = wants.iter().fold(true, |all, want| {
        lumit_media::web::frames::ahead(&want.source, want.frame, want.target_width) && all
    });
    let Some(playback) = state.playback.as_mut() else {
        return false;
    };
    if landed || playback.clock_reached(frame) {
        return false;
    }
    // Back to be rendered again next turn. A landing wakes the worker.
    playback.next = frame;
    playback.ended = false;
    crate::turns::pause(std::time::Duration::from_millis(4));
    true
}

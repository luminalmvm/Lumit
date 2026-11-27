//! The one frame the user is waiting for: its progress bar and what it cost.

use super::*;

/// Translate one measured frame for the frontend (docs/13 §7.1). Ids cross as
/// strings because that is how every other reference does — the frontend
/// matches them against the ids its read model already holds.
#[frb(ignore)]
pub(super) fn profile_of(
    p: &lumit_render::profile::FrameProfile,
    view: u32,
) -> crate::api::state::BridgeFrameProfile {
    let stage = |s: lumit_render::profile::RenderStage| f64::from(p.stage_ms[s.code() as usize]);
    crate::api::state::BridgeFrameProfile {
        frame: p.frame,
        total_ms: f64::from(p.total_ms),
        plan_ms: stage(lumit_render::profile::RenderStage::Planning),
        decode_ms: stage(lumit_render::profile::RenderStage::Decoding),
        build_ms: stage(lumit_render::profile::RenderStage::Building),
        composite_ms: stage(lumit_render::profile::RenderStage::Compositing),
        present_ms: stage(lumit_render::profile::RenderStage::Presenting),
        layers: p
            .layers
            .iter()
            .map(|l| crate::api::state::BridgeLayerTiming {
                layer: l.layer.to_string(),
                ms: f64::from(l.ms),
                effects: l
                    .effects
                    .iter()
                    .map(|e| crate::api::state::BridgeEffectTiming {
                        effect: e.effect.to_string(),
                        ms: f64::from(e.ms),
                    })
                    .collect(),
            })
            .collect(),
        view,
    }
}

/// Render the one frame the user is waiting for, with the bar running.
///
/// The switches are turned on for this frame and off again straight after, so
/// **nothing else the worker renders is watched or measured** — not playback,
/// not the idle cache fill, not a scope trace. That is the whole rule
/// (docs/07 §2.5: the bar never appears during playback), and keeping it here
/// rather than in each render path is what stops a later caller forgetting it.
/// The one other measured render is [`measure_pending`]'s: the same frame
/// this one asked for, composited again in an idle moment because a tier
/// served it without numbers.
///
/// The closing report is sent whatever happened inside, including a frame that
/// faulted or one served straight from the cache with no report at all: a bar
/// that was started must always be ended.
///
/// `measure` is false for a live drag's frame. Measuring waits for the card
/// at every layer and every effect, which on a slow card is most of a drag
/// tick, and a drag frame is provisional anyway: the release commits and the
/// committed frame comes back through here measured, so the Time column
/// reads the frame that stays rather than one that was already replaced.
#[frb(ignore)]
pub(super) fn watched<R>(
    state: &mut WorkerState,
    stream: &mut WorkerResponseStream,
    frame: u64,
    measure: bool,
    render: impl FnOnce(&mut WorkerState, &mut WorkerResponseStream) -> R,
) -> R {
    state.renderer.watch_frames(true);
    state
        .renderer
        .measure_frames(measure && crate::profiling::wanted());
    let out = render(state, stream);
    state.renderer.watch_frames(false);
    state.renderer.measure_frames(false);
    _ = stream.add(WorkerResponse::RenderProgress(
        crate::api::state::BridgeRenderProgress {
            frame,
            stage: lumit_render::profile::RenderStage::Presenting.code(),
            fraction: 1.0,
            done: true,
            view: state.view_id(),
        },
    ));
    out
}

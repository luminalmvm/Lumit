//! Working out what to decode before decoding it.
//!
//! # In plain terms
//!
//! Compositing a frame needs pixels, and pixels come from video files, which are
//! slow to read. So the pipeline never decodes speculatively: it first walks the
//! comp at the wanted moment and writes down exactly which layer needs which
//! frame of which file, at what width — the **decode plan** ([`CompJob`] per
//! layer). Nested comps are walked too (at their own mapped times), matte
//! sources and effect layer-inputs are included even though they are usually
//! hidden, and a temporal effect like echo asks for its neighbour frames here
//! rather than surprising the decoder later.
//!
//! Planning is pure and cheap — it opens no files and touches no GPU. That
//! matters twice over: the plan doubles as the honest statement of what a frame
//! depends on, and it can be re-run freely while a value is being dragged to
//! notice that *nothing about the decode changed*, so the already-decoded pixels
//! can be reused.
//!
//! [`Quality`] is the other half: how wide to decode. Full resolution is rarely
//! wanted for a preview, and a source decoded at viewport size is several times
//! cheaper than one decoded at 4K and thrown away.

use crate::decode::CompJob;
use crate::source::SourceProbes;
use lumit_core::model::{Composition, Document, LayerKind, ProjectItem};
use std::path::PathBuf;
use uuid::Uuid;

/// While the user is actively scrubbing or dragging, footage decodes at most
/// this wide so a frame comes back fast (the specified resolution reloads the
/// moment they stop). Chosen to keep even 4K sources instant to draft.
pub const DRAFT_MAX_WIDTH: u32 = 640;

/// How coarsely to decode this preview — the quality axis of both the decode
/// plan and the frame-cache key. Keeping the two in one type is deliberate: if
/// they could disagree, a frame decoded at one width could be served from a
/// cache entry filed under another.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quality {
    /// Scrub/drag draft: cap the decode width hard for instant feedback. Never
    /// raises the width the settings below ask for, only lowers it.
    pub draft: bool,
    /// Auto resolution: decode at the size the frame is actually displayed,
    /// never above native however far the view is zoomed in.
    pub auto_res: bool,
    /// The on-screen scale of the Viewer, used by `auto_res`.
    pub display_scale: f32,
    /// Manual preview-resolution divisor: 1 = Full, 2 = Half, 3 = Third,
    /// 4 = Quarter (docs/01-GLOSSARY.md §5).
    pub divisor: u32,
}

impl Default for Quality {
    /// Full resolution, no draft — what export and a still Viewer want.
    fn default() -> Self {
        Self {
            draft: false,
            auto_res: false,
            display_scale: 1.0,
            divisor: 1,
        }
    }
}

impl Quality {
    /// One decode-width policy for requests AND cache keys — if these ever
    /// disagreed, a cached frame could present at the wrong resolution. `None`
    /// means "decode at native width".
    #[must_use]
    pub fn target_width(self, natural_w: u32) -> Option<u32> {
        let specified = if self.auto_res {
            let scale = self.display_scale.clamp(0.05, 1.0);
            let w = (natural_w as f32 * scale).round() as u32;
            (w < natural_w).then_some(w.max(16))
        } else {
            (self.divisor > 1).then(|| natural_w / self.divisor)
        };
        if self.draft {
            // Never coarser than needed: cap the specified width, never raise it.
            let w = specified.unwrap_or(natural_w).min(DRAFT_MAX_WIDTH);
            return (w < natural_w).then_some(w.max(16));
        }
        specified
    }

    /// The number the frame-cache key folds in, so each resolution tier keys
    /// separately (docs/06 §5.2 quality axis). Auto folds the live zoom in the
    /// same way, at 1% granularity.
    #[must_use]
    pub fn tag(self) -> u32 {
        if self.auto_res {
            1000 + (self.display_scale.clamp(0.05, 1.0) * 100.0) as u32
        } else {
            self.divisor
        }
    }
}

/// A live Retime override for one layer, so a "Time" drag decodes the dragged
/// source frame rather than the committed one. Unlike a transform or effect
/// drag, retiming changes *which* frame is shown, so it cannot be handled by
/// re-compositing already-decoded pixels — it has to reach the plan.
pub struct RetimeOverride {
    pub layer: Uuid,
    pub retime: lumit_core::retime::Retime,
}

/// The inputs a plan walk carries down the Precomp recursion, unchanged at
/// every depth: what the media is, how coarsely to decode, and any live Retime
/// drag. Bundled so the recursive walk stays readable.
pub struct PlanContext<'a> {
    pub doc: &'a Document,
    pub quality: Quality,
    pub probes: &'a dyn SourceProbes,
    pub retime_override: Option<&'a RetimeOverride>,
}

/// Recursively collect the decode jobs comp `comp` needs at comp time `t`
/// (docs/06-RENDER-PIPELINE.md: Precomp evaluation). Cycle-guarded through
/// `visited`, which must already contain `comp.id`.
///
/// The context's `probes` answers what each footage item is; an unprobed item
/// contributes no job at all and is retried once its probe lands.
pub fn collect_comp_jobs(
    ctx: &PlanContext<'_>,
    comp: &Composition,
    t: f64,
    jobs: &mut Vec<CompJob>,
    visited: &mut Vec<Uuid>,
) {
    let PlanContext {
        doc,
        quality,
        probes,
        retime_override,
    } = *ctx;
    let in_span =
        |l: &lumit_core::model::Layer| t >= l.in_point.0.to_f64() && t < l.out_point.0.to_f64();
    let mut wanted: Vec<Uuid> = Vec::new();
    for l in &comp.layers {
        if l.switches.visible && in_span(l) {
            wanted.push(l.id);
            if let Some(m) = &l.matte {
                if !wanted.contains(&m.layer) {
                    wanted.push(m.layer);
                }
            }
            // Layer-input references (K-123, e.g. a DoF depth pass) decode
            // exactly like matte sources: the referenced layer is usually
            // hidden (you don't want the depth map rendering), but its
            // pixels still feed the effect.
            for e in l.effects.iter().filter(|e| e.enabled) {
                for p in &e.params {
                    if let lumit_core::model::EffectValue::Layer(Some(id)) = p.value {
                        if !wanted.contains(&id) {
                            wanted.push(id);
                        }
                    }
                }
            }
        }
    }
    // Posterize Time (docs/08 §3.25, FX-1): a layer covered by a live
    // Posterize decodes its source at the held grid time, not the live
    // playhead, so footage playback visibly steps — the decode twin of the
    // held re-render the draw builder performs. `sample_times[idx]` is the
    // held comp time for `comp.layers[idx]`; equal to `t` for every layer
    // when no Posterize is live, so an ordinary comp is unchanged.
    let sample_times = lumit_core::fx::posterize_sample_times(&comp.layers, t);
    for (idx, layer) in comp.layers.iter().enumerate() {
        if !wanted.contains(&layer.id) || !in_span(layer) {
            continue;
        }
        let lt = sample_times[idx] - layer.start_offset.0.to_f64();
        match &layer.kind {
            // No footage source to decode (an adjustment layer processes
            // the composite below; solids/text/cameras rasterise elsewhere).
            LayerKind::Solid { .. }
            | LayerKind::Text { .. }
            | LayerKind::Camera { .. }
            | LayerKind::Adjustment => {}
            LayerKind::NullObject => {}
            LayerKind::Sequence { clips } => {
                // Resolve the clip under the playhead to a footage frame
                // (comp-source clips + gaps are handled elsewhere/skip).
                if let Some((_id, lumit_core::sequence::ClipSource::Footage(item), st)) =
                    lumit_core::sequence::resolve(clips, lt)
                {
                    let (Some(ProjectItem::Footage(f)), Some((fps, nat_w, nat_h, src_frames))) =
                        (doc.item(item), probes.probe(item).video())
                    else {
                        continue;
                    };
                    use lumit_core::retime::Interpolation;
                    let interp = lumit_core::sequence::active_clip(clips, lt)
                        .map(|c| c.interpolation.clone());
                    let blend_on =
                        matches!(interp, Some(Interpolation::Blend | Interpolation::Flow(_)));
                    let flow = matches!(interp, Some(Interpolation::Flow(_)));
                    let flow_full =
                        matches!(&interp, Some(Interpolation::Flow(p)) if !p.half_resolution);
                    let sample_fps = match &interp {
                        Some(Interpolation::Flow(p)) => p.input_fps_at(lt),
                        _ => None,
                    };
                    let (source_frame, blend) =
                        lumit_core::pixels::frame_pick(st, fps, src_frames, blend_on, sample_fps);
                    jobs.push(CompJob {
                        layer: layer.id,
                        item,
                        path: PathBuf::from(&f.media.absolute_path),
                        source_frame,
                        target_width: quality.target_width(nat_w),
                        natural_w: nat_w,
                        natural_h: nat_h,
                        blend,
                        flow,
                        flow_full,
                        // Temporal effects on Sequence clips are a later
                        // refinement (clip-relative neighbour resolution);
                        // footage layers first.
                        temporal: Vec::new(),
                        flow_neighbour: None,
                        slate: false,
                    });
                }
            }
            LayerKind::Precomp { comp: nested_id } => {
                if visited.contains(nested_id) {
                    continue; // cycle guard
                }
                if let Some(nested) = doc.comp(*nested_id) {
                    visited.push(*nested_id);
                    collect_comp_jobs(ctx, nested, lt, jobs, visited);
                    visited.pop();
                }
            }
            LayerKind::Footage { item, retime } => {
                // A live "Time" drag overrides this layer's retime so the
                // decode picks the dragged source frame (the frame itself
                // changes, unlike a transform/effect live patch).
                let live_retime;
                let retime: &Option<lumit_core::retime::Retime> = match retime_override {
                    Some(o) if o.layer == layer.id => {
                        live_retime = Some(o.retime.clone());
                        &live_retime
                    }
                    _ => retime,
                };
                let Some(ProjectItem::Footage(f)) = doc.item(*item) else {
                    continue;
                };
                let probe = probes.probe(*item);
                // Missing media still draws (docs/07 §3.3): a slate job at
                // comp size, so the layer shows test bars in place of the
                // picture instead of silently vanishing. Sized to the comp
                // because a file we cannot open has no size to report.
                if probe.slates() {
                    jobs.push(CompJob {
                        layer: layer.id,
                        item: *item,
                        path: PathBuf::from(&f.media.absolute_path),
                        source_frame: 0,
                        target_width: None,
                        natural_w: comp.width,
                        natural_h: comp.height,
                        blend: None,
                        flow: false,
                        flow_full: false,
                        temporal: Vec::new(),
                        flow_neighbour: None,
                        slate: true,
                    });
                    continue;
                }
                // Not probed yet, or audio-only: no picture. Retried once the
                // probe lands.
                let Some((fps, nat_w, nat_h, src_frames)) = probe.video() else {
                    continue;
                };
                // Retime maps local time → source time before frame pick; the
                // layer decides which map answers (K-197: the keyframable
                // property, else the segment store). Its interpolation policy
                // decides nearest vs blend.
                let source_at = |t: f64| match retime {
                    // A live "Time" drag is the segment store by construction,
                    // so an override speaks for itself.
                    Some(r) if retime_override.is_some_and(|o| o.layer == layer.id) => {
                        r.evaluate(t)
                    }
                    _ => layer.source_time_at(t),
                };
                let source_time = source_at(lt);
                use lumit_core::retime::Interpolation;
                let interp = retime.as_ref().map(|r| &r.interpolation);
                let blend_on =
                    matches!(interp, Some(Interpolation::Blend | Interpolation::Flow(_)));
                let flow = matches!(interp, Some(Interpolation::Flow(_)));
                let flow_full =
                    matches!(interp, Some(Interpolation::Flow(p)) if !p.half_resolution);
                let sample_fps = match interp {
                    Some(Interpolation::Flow(p)) => p.input_fps_at(lt),
                    _ => None,
                };
                let (source_frame, blend) = lumit_core::pixels::frame_pick(
                    source_time,
                    fps,
                    src_frames,
                    blend_on,
                    sample_fps,
                );
                // Neighbour source frames for a temporal effect stack
                // (echo/trails, flow motion blur, datamosh): the layer's
                // source at each non-zero offset in the stack's window,
                // mapped through the retime like the primary frame. Empty
                // unless the stack actually reads other frames, so a plain
                // footage layer decodes exactly one frame.
                let temporal =
                    if lumit_core::fx::stack_is_temporal(&layer.effects, layer.switches.fx) {
                        let comp_dt = 1.0 / comp.frame_rate.fps().max(1.0);
                        lumit_core::fx::stack_temporal_window(&layer.effects, layer.switches.fx)
                            .into_iter()
                            .filter(|&o| o != 0)
                            .map(|o| {
                                let nlt = lt + f64::from(o) * comp_dt;
                                let nst = source_at(nlt);
                                let (nf, _) = lumit_core::pixels::frame_pick(
                                    nst, fps, src_frames, false, None,
                                );
                                (o, nf)
                            })
                            .collect()
                    } else {
                        Vec::new()
                    };
                jobs.push(CompJob {
                    layer: layer.id,
                    item: *item,
                    path: PathBuf::from(&f.media.absolute_path),
                    source_frame,
                    target_width: quality.target_width(nat_w),
                    natural_w: nat_w,
                    natural_h: nat_h,
                    blend,
                    flow,
                    flow_full,
                    temporal,
                    // Flow motion blur / Datamosh measure motion between
                    // this frame and their requested neighbour (already
                    // in `temporal`).
                    flow_neighbour: lumit_core::fx::stack_flow_neighbour(
                        &layer.effects,
                        layer.switches.fx,
                    ),
                    slate: false,
                });
            }
        }
    }
}

/// The decode plan for one comp frame: the convenience wrapper around
/// [`collect_comp_jobs`] that sets up the cycle guard.
#[must_use]
pub fn plan_comp_frame(
    doc: &Document,
    comp: &Composition,
    t: f64,
    quality: Quality,
    probes: &dyn SourceProbes,
    retime_override: Option<&RetimeOverride>,
) -> Vec<CompJob> {
    let ctx = PlanContext {
        doc,
        quality,
        probes,
        retime_override,
    };
    let mut jobs = Vec::new();
    let mut visited = vec![comp.id];
    collect_comp_jobs(&ctx, comp, t, &mut jobs, &mut visited);
    jobs
}

/// Whether two decode plans ask for exactly the same pixels — the test a live
/// value drag runs to decide it can re-composite from the frame it already has
/// instead of decoding again.
///
/// Only the identity of the wanted pixels is compared (which layer, which item,
/// which source frame, at what width, with which neighbours and blend partner),
/// never the placement or effects — those are precisely what the drag is
/// changing, and re-running them is the cheap half.
#[must_use]
pub fn same_decode(a: &[CompJob], b: &[CompJob]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| {
            x.layer == y.layer
                && x.item == y.item
                && x.source_frame == y.source_frame
                && x.target_width == y.target_width
                && x.slate == y.slate
                && x.blend == y.blend
                && x.flow == y.flow
                && x.flow_full == y.flow_full
                && x.temporal == y.temporal
                && x.flow_neighbour == y.flow_neighbour
        })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// Full quality decodes at native width; a divisor and Auto both shrink it,
    /// and draft caps on top without ever raising the specified width. This is
    /// the policy both the decode request and the cache key read, so a bug here
    /// would file a frame under the wrong resolution.
    #[test]
    fn target_width_follows_the_quality_policy() {
        let full = Quality::default();
        assert_eq!(full.target_width(1920), None, "Full decodes at native");

        let half = Quality {
            divisor: 2,
            ..Quality::default()
        };
        assert_eq!(half.target_width(1920), Some(960));

        let auto = Quality {
            auto_res: true,
            display_scale: 0.25,
            ..Quality::default()
        };
        assert_eq!(auto.target_width(1920), Some(480));

        // Auto never decodes ABOVE native, however far the view is zoomed in.
        let zoomed = Quality {
            auto_res: true,
            display_scale: 4.0,
            ..Quality::default()
        };
        assert_eq!(zoomed.target_width(1920), None);

        // A source already finer than every setting decodes natively.
        let half_of_small = Quality {
            auto_res: true,
            display_scale: 0.5,
            ..Quality::default()
        };
        assert_eq!(half_of_small.target_width(1000), Some(500));

        // Draft caps a large source hard...
        let draft = Quality {
            draft: true,
            ..Quality::default()
        };
        assert_eq!(draft.target_width(3840), Some(DRAFT_MAX_WIDTH));
        assert_eq!(draft.target_width(1920), Some(DRAFT_MAX_WIDTH));
        // ...still caps when the specified width (960) is above the cap...
        let draft_half = Quality {
            draft: true,
            divisor: 2,
            ..Quality::default()
        };
        assert_eq!(draft_half.target_width(1920), Some(DRAFT_MAX_WIDTH));
        // ...but never RAISES an already-coarser specified width.
        let draft_quarter = Quality {
            draft: true,
            divisor: 4,
            ..Quality::default()
        };
        assert_eq!(draft_quarter.target_width(1920), Some(480));
        assert_eq!(draft_quarter.target_width(1280), Some(320));
        // Auto zoomed right out stays where Auto put it, under draft too.
        let draft_auto = Quality {
            draft: true,
            auto_res: true,
            display_scale: 0.1,
            ..Quality::default()
        };
        assert_eq!(draft_auto.target_width(1920), Some(192));
        // A source already smaller than the cap needs no draft decode at all.
        assert_eq!(draft.target_width(320), None);
    }

    /// The cache-key tag separates the resolution tiers, and Auto's tag moves
    /// with the zoom so a frame decoded at one zoom is never served at another.
    #[test]
    fn the_quality_tag_separates_tiers() {
        let full = Quality::default();
        let half = Quality {
            divisor: 2,
            ..Quality::default()
        };
        assert_ne!(full.tag(), half.tag());
        let auto_a = Quality {
            auto_res: true,
            display_scale: 0.5,
            ..Quality::default()
        };
        let auto_b = Quality {
            auto_res: true,
            display_scale: 0.25,
            ..Quality::default()
        };
        assert_ne!(auto_a.tag(), auto_b.tag());
        assert_ne!(auto_a.tag(), full.tag(), "Auto never collides with manual");
    }

    fn job(layer: Uuid, item: Uuid, source_frame: usize) -> CompJob {
        CompJob {
            layer,
            item,
            path: PathBuf::from("a.mp4"),
            source_frame,
            target_width: None,
            natural_w: 8,
            natural_h: 8,
            blend: None,
            flow: false,
            flow_full: false,
            temporal: Vec::new(),
            flow_neighbour: None,
            slate: false,
        }
    }

    /// Two plans wanting the same pixels compare equal — the green light for a
    /// live drag to skip decoding — and a different source frame does not.
    #[test]
    fn same_decode_compares_the_wanted_pixels_only() {
        let (l, i) = (Uuid::now_v7(), Uuid::now_v7());
        assert!(same_decode(&[job(l, i, 5)], &[job(l, i, 5)]));
        assert!(
            !same_decode(&[job(l, i, 5)], &[job(l, i, 6)]),
            "a different source frame must force a decode"
        );
        assert!(
            !same_decode(&[job(l, i, 5)], &[]),
            "a layer appearing or leaving must force a decode"
        );
        // A different decode width is a different pixel buffer.
        let mut wide = job(l, i, 5);
        wide.target_width = Some(640);
        assert!(!same_decode(&[job(l, i, 5)], &[wide]));
    }
}

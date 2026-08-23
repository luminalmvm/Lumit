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
    /// The display scale as the cache sees it: rounded down to the same 1% step
    /// [`Self::tag`] keys by.
    ///
    /// **Both of them must use this, or footage stops being nameable.** The tag
    /// declares that two scales inside the same 1% are the same quality, and a
    /// solid obeys that, because the tag is all a solid's name folds in. Footage
    /// also folds in the width it is decoded at — and that came from the raw
    /// scale, so 0.4235 and 0.4240 decoded to 813 and 814 pixels and gave the
    /// same frame two different names.
    ///
    /// The cache bar is where that showed. It asks by a scale it has rounded to
    /// a thousandth, which is nearly never the exact float the render used, so
    /// the bar named every frame differently from the way it was banked and drew
    /// an empty stripe over a composition that was fully cached and playing. A
    /// composition of solids was unaffected, which is what made it look like a
    /// fault in footage.
    ///
    /// Rounding here rather than at each caller keeps the decode and the name in
    /// step by construction: the width in the name is the width the pixels were
    /// decoded at, whoever asked. It also stops a window resize from re-decoding
    /// for a scale change too small to see.
    ///
    /// The scale is taken to the nearest thousandth **first**, because that is
    /// the form the cache bar asks in (`scale_q`, docs/06 §5.6) while a scrub
    /// asks with the raw float. Flooring the raw float straight to a 1% step
    /// put about one scale in twenty on a different step from its own
    /// thousandth — 0.4296 floors to 42%, its thousandth 0.430 to 43% — so the
    /// bar named those frames differently from the way they were banked and
    /// drew them empty. Integer arithmetic throughout, so naming a scale and
    /// naming its thousandth give the same answer by construction.
    #[must_use]
    pub fn keyed_scale(self) -> f32 {
        let thousandths = (self.display_scale.clamp(0.05, 1.0) * 1000.0).round() as u32;
        (thousandths / 10) as f32 / 100.0
    }

    /// One decode-width policy for requests AND cache keys — if these ever
    /// disagreed, a cached frame could present at the wrong resolution. `None`
    /// means "decode at native width".
    #[must_use]
    pub fn target_width(self, natural_w: u32) -> Option<u32> {
        let specified = if self.auto_res {
            let w = (natural_w as f32 * self.keyed_scale()).round() as u32;
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
            1000 + (self.keyed_scale() * 100.0).round() as u32
        } else {
            self.divisor
        }
    }
}

/// The nested-frame question a plan asks (K-422, [`PlanContext::held`]):
/// "is this comp's frame at this layer time already a finished texture?"
pub type HeldNested<'a> = &'a dyn Fn(&Composition, f64) -> bool;

/// The inputs a plan walk carries down the Precomp recursion, unchanged at
/// every depth: what the media is and how coarsely to decode. Bundled so the
/// recursive walk stays readable.
pub struct PlanContext<'a> {
    pub doc: &'a Document,
    pub quality: Quality,
    pub probes: &'a dyn SourceProbes,
    /// Answers "is this nested comp's frame, at this layer time, already held
    /// as a finished texture?" (K-422). When it is, the planner asks for none
    /// of that comp's decodes: the realiser will serve the texture and never
    /// look at the pixels. This is the one place planning knows a cache exists
    /// — the module header's "pure and cheap" is kept by making it a question
    /// the caller answers, and the coupling is worth it because without it a
    /// held Precomp still cost every source decode inside it, which is most of
    /// what rendering a Precomp costs. The answerer must *hold* what it says
    /// yes to until the frame is realised (`FxCache::pin_nested`), or a yes
    /// here becomes a nested comp realised from no pixels.
    pub held: Option<HeldNested<'a>>,
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
        held,
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
        let lt = lumit_core::time::layer_time(sample_times[idx], layer.start_offset.0);
        match &layer.kind {
            // No footage source to decode (an adjustment layer processes
            // the composite below; solids/text/cameras rasterise elsewhere).
            LayerKind::Solid { .. }
            | LayerKind::Text { .. }
            | LayerKind::Shape { .. }
            | LayerKind::Camera { .. }
            | LayerKind::Light { .. }
            | LayerKind::Adjustment
            | LayerKind::Null => {}
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
                    let clip = lumit_core::sequence::active_clip(clips, lt);
                    // Same engagement gate as a Footage layer (K-331); the
                    // clip's own retime supplies the speed.
                    let comp_fps = comp.frame_rate.fps();
                    let flow = match clip.map(|c| (&c.interpolation, c.retime.as_ref())) {
                        Some((Interpolation::Flow(p), retime)) => {
                            let speed = lumit_core::retime::property_speed_at(retime, lt);
                            p.engages(p.read_fps_at(lt, fps), comp_fps, speed)
                                .then(|| p.clone())
                        }
                        _ => None,
                    };
                    let blend_on =
                        matches!(clip.map(|c| &c.interpolation), Some(Interpolation::Blend))
                            || flow.is_some();
                    let sample_fps = flow.as_ref().and_then(|p| p.input_fps_at(lt));
                    let target_width = if flow.is_some() {
                        None // flow decodes natively (K-331)
                    } else {
                        quality.target_width(nat_w)
                    };
                    let (source_frame, blend) =
                        lumit_core::pixels::frame_pick(st, fps, src_frames, blend_on, sample_fps);
                    jobs.push(CompJob {
                        layer: layer.id,
                        item,
                        path: PathBuf::from(&f.media.absolute_path),
                        source_frame,
                        target_width,
                        natural_w: nat_w,
                        natural_h: nat_h,
                        blend,
                        flow,
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
                    // A held nested frame wants no decodes (K-422). Asked only
                    // where the builder will ask by the same name: at the live
                    // time (a Posterize-held layer is built at another time,
                    // by a walk with no keyer) and for a Precomp that is not
                    // collapsed (a collapsed one is spliced in, never named).
                    let live = sample_times[idx] == t;
                    let collapsed = matches!(
                        lumit_core::model::collapse_state(doc, comp, layer, lt),
                        lumit_core::model::CollapseState::Active
                    );
                    if live && !collapsed && held.is_some_and(|held| held(nested, lt)) {
                        continue;
                    }
                    visited.push(*nested_id);
                    collect_comp_jobs(ctx, nested, lt, jobs, visited);
                    visited.pop();
                }
            }
            LayerKind::Footage { item } => {
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
                        flow: None,
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
                // Retime maps local time → source time before the frame pick
                // (K-249: the layer's own property is the only map). Its
                // interpolation policy, which sits beside that map rather than
                // inside it, decides nearest vs blend.
                let source_time = layer.source_time_at(lt);
                use lumit_core::retime::Interpolation;
                // Flow only engages where it can help (K-088, built in K-331):
                // at 100% or faster every comp frame lands on a source frame,
                // so there is no in-between frame to invent and the policy
                // degrades to Nearest. `always` overrides.
                let speed = lumit_core::retime::property_speed_at(layer.retime.as_ref(), lt);
                let comp_fps = comp.frame_rate.fps();
                let flow = match &layer.interpolation {
                    Interpolation::Flow(p) => p
                        .engages(p.read_fps_at(lt, fps), comp_fps, speed)
                        .then(|| p.clone()),
                    _ => None,
                };
                let interp = &layer.interpolation;
                let blend_on = matches!(interp, Interpolation::Blend) || flow.is_some();
                let sample_fps = flow.as_ref().and_then(|p| p.input_fps_at(lt));
                let flow_neighbour =
                    lumit_core::fx::stack_flow_neighbour(&layer.effects, layer.switches.fx);
                // A layer that needs flow decodes at its own width whatever the
                // preview tier says (K-331): flow measured on a shrunk decode is
                // a different measurement, not the same one smaller. Must match
                // `Stamper::stamp`'s `native` exactly, or the frame's name lies
                // about the width of the pixels in it.
                let native = flow.is_some() || flow_neighbour.is_some();
                let target_width = if native {
                    None
                } else {
                    quality.target_width(nat_w)
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
                                let nst = layer.source_time_at(nlt);
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
                    target_width,
                    natural_w: nat_w,
                    natural_h: nat_h,
                    blend,
                    flow,
                    temporal,
                    // Flow motion blur / Datamosh measure motion between
                    // this frame and their requested neighbour (already
                    // in `temporal`).
                    flow_neighbour,
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
) -> Vec<CompJob> {
    plan_comp_frame_held(doc, comp, t, quality, probes, None)
}

/// [`plan_comp_frame`] with the nested-frame answerer (K-422,
/// [`PlanContext::held`]).
#[must_use]
pub fn plan_comp_frame_held(
    doc: &Document,
    comp: &Composition,
    t: f64,
    quality: Quality,
    probes: &dyn SourceProbes,
    held: Option<HeldNested<'_>>,
) -> Vec<CompJob> {
    let ctx = PlanContext {
        doc,
        quality,
        probes,
        held,
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
                && x.temporal == y.temporal
                && x.flow_neighbour == y.flow_neighbour
        })
}

impl CompJob {
    /// The content name of the pixels this job decodes (K-421): a hash of
    /// exactly the fields [`same_decode`] compares, so two jobs that would
    /// decode the same pixels have the same name. The layer id is left out on
    /// purpose — a name is about content, never about which row asked — and
    /// the flow settings go in as their serialised form, which is stable across
    /// runs where a struct's bytes would not be.
    #[must_use]
    pub fn source_key(&self) -> u128 {
        let mut h = blake3::Hasher::new();
        h.update(b"decode/1/");
        h.update(self.item.as_bytes());
        h.update(self.path.to_string_lossy().as_bytes());
        h.update(&self.source_frame.to_le_bytes());
        h.update(&self.target_width.unwrap_or(u32::MAX).to_le_bytes());
        h.update(&[u8::from(self.slate)]);
        if let Some((ceil, weight)) = self.blend {
            h.update(&ceil.to_le_bytes());
            h.update(&weight.to_le_bytes());
        }
        if let Some(flow) = &self.flow {
            h.update(b"flow/");
            h.update(&bincode::serialize(flow).unwrap_or_default());
        }
        for (offset, frame) in &self.temporal {
            h.update(&offset.to_le_bytes());
            h.update(&frame.to_le_bytes());
        }
        h.update(&self.flow_neighbour.unwrap_or(i32::MIN).to_le_bytes());
        let mut k = [0u8; 16];
        k.copy_from_slice(&h.finalize().as_bytes()[..16]);
        u128::from_le_bytes(k)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// **A scale and its thousandth are named the same** — the scrub asks with
    /// the raw float, the cache bar with the scale rounded to a thousandth, and
    /// a frame banked by one must be found by the other. Flooring the raw float
    /// to 1% put about one scale in twenty on a different step from its own
    /// thousandth; fails without the quantise-first step.
    #[test]
    fn keyed_scale_agrees_with_its_own_thousandth() {
        let quality = |scale: f32| Quality {
            auto_res: true,
            display_scale: scale,
            ..Quality::default()
        };
        let mut s = 0.05_f32;
        while s <= 1.0 {
            let rounded = (s * 1000.0).round() / 1000.0;
            assert_eq!(
                quality(s).keyed_scale(),
                quality(rounded).keyed_scale(),
                "scale {s} and its thousandth {rounded} must key alike"
            );
            assert_eq!(quality(s).tag(), quality(rounded).tag());
            s += 0.0001;
        }
    }

    /// **Two scales the tag calls equal must decode to the same width.**
    ///
    /// The regression: the tag keys Auto at 1% steps, so 0.4235 and 0.4240 are
    /// one quality — but the decode width came from the raw float and gave 813
    /// and 814 pixels. Footage folds that width into its name, thus one frame
    /// got two names; a solid folds in only the tag, thus it got one. The cache
    /// bar reads by a scale rounded to a thousandth, which is almost never the
    /// float the render used, so it named every frame differently from the way
    /// it was banked and drew nothing over a composition that was fully cached
    /// and playing.
    #[test]
    fn one_quality_step_is_one_decode_width() {
        let at = |scale: f32| Quality {
            auto_res: true,
            display_scale: scale,
            ..Quality::default()
        };
        // Inside one 1% step, whatever the float.
        for (a, b) in [(0.4235f32, 0.424f32), (0.4237, 0.424), (0.4271, 0.427)] {
            assert_eq!(
                at(a).tag(),
                at(b).tag(),
                "the tag already calls {a} and {b} one quality"
            );
            assert_eq!(
                at(a).target_width(1920),
                at(b).target_width(1920),
                "thus they must decode to one width, or footage gets two names"
            );
        }
        // And a step that IS a step still separates them.
        assert_ne!(at(0.42).tag(), at(0.43).tag());
        assert_ne!(at(0.42).target_width(1920), at(0.43).target_width(1920));
    }

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
            flow: None,
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

    /// **Footage inside a precomp that is referenced only as a matte still gets
    /// decoded** (K-268).
    ///
    /// K-266 recorded this as an open boundary — "the decode planner never
    /// visits a matte-only precomp" — and taught the draw builder to render the
    /// nested comp anyway. The planner turned out to walk the reference
    /// already: a matte source and a layer-input reference are both `wanted`
    /// whether or not the layer is visible, and a Precomp among them recurses.
    /// So the boundary was the *draw* side, which K-266 and K-268 have now
    /// closed at both ends — and this test is what keeps the planner honest, so
    /// nobody has to re-derive it from a black matte.
    ///
    /// Both shapes of reference are checked: the track matte (`Layer::matte`)
    /// and the layer-input parameter a flare's Matte source or a DoF depth pass
    /// uses.
    #[test]
    fn a_matte_only_precomp_still_decodes_its_footage() {
        use lumit_core::model::{
            Composition, Document, EffectInstance, EffectKey, EffectNamespace, EffectParam,
            EffectValue, FootageItem, Layer, LayerKind, LinearColour, MatteChannel, MatteRef,
            MediaRef, Switches, TransformGroup,
        };
        use lumit_core::time::{CompTime, Duration, FrameRate, Rational};
        use std::collections::HashMap;

        let layer = |kind: LayerKind| Layer {
            markers: Vec::new(),
            id: Uuid::now_v7(),
            name: "l".into(),
            kind,
            in_point: CompTime(Rational::ZERO),
            out_point: CompTime(Rational::new(10, 1).unwrap()),
            start_offset: CompTime(Rational::ZERO),
            transform: TransformGroup::default(),
            matte: None,
            parent: None,
            label: 0,
            volume_db: lumit_core::anim::Property::zero(),
            retime: None,
            interpolation: lumit_core::retime::Interpolation::default(),
            parked_flow: None,
            blend: lumit_core::model::BlendMode::default(),
            masks: Vec::new(),
            paint: Vec::new(),
            effects: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        };
        let comp = |layers: Vec<Layer>| Composition {
            id: Uuid::now_v7(),
            name: "c".into(),
            width: 64,
            height: 64,
            frame_rate: FrameRate::new(60, 1).unwrap(),
            duration: Duration(Rational::new(10, 1).unwrap()),
            background: LinearColour::BLACK,
            work_area: None,
            layers,
            markers: Vec::new(),
            motion_blur: lumit_core::model::MotionBlur::default(),
            extra: serde_json::Map::new(),
        };

        // Two ways of pointing at the same hidden precomp, one per case.
        for track_matte in [true, false] {
            let mut doc = Document::new();
            let item = Uuid::now_v7();
            doc.items.push(ProjectItem::Footage(FootageItem {
                id: item,
                name: "f".into(),
                media: MediaRef {
                    relative_path: "f.mp4".into(),
                    absolute_path: "/f.mp4".into(),
                    fingerprint: None,
                    extra: serde_json::Map::new(),
                },
                extra: serde_json::Map::new(),
            }));
            let inner = comp(vec![layer(LayerKind::Footage { item })]);
            let inner_id = inner.id;
            doc.items.push(ProjectItem::Composition(inner));

            // The precomp is hidden — a matte source always is.
            let mut matte_layer = layer(LayerKind::Precomp { comp: inner_id });
            matte_layer.switches.visible = false;
            let mut consumer = layer(LayerKind::Solid {
                def: Uuid::now_v7(),
            });
            if track_matte {
                consumer.matte = Some(MatteRef {
                    layer: matte_layer.id,
                    channel: MatteChannel::Alpha,
                    inverted: false,
                    source: lumit_core::model::LayerInputSource::default(),
                });
            } else {
                consumer.effects.push(EffectInstance {
                    id: Uuid::now_v7(),
                    effect: EffectKey {
                        namespace: EffectNamespace::Builtin,
                        match_name: "lens_flare".into(),
                        version: 1,
                        extra: serde_json::Map::new(),
                    },
                    enabled: true,
                    params: vec![
                        EffectParam {
                            id: "source_type".into(),
                            value: EffectValue::Choice(1),
                            extra: serde_json::Map::new(),
                        },
                        EffectParam {
                            id: "matte".into(),
                            value: EffectValue::Layer(Some(matte_layer.id)),
                            extra: serde_json::Map::new(),
                        },
                    ],
                    sample_temporally: true,
                    custom_name: None,
                    extra: serde_json::Map::new(),
                });
            }
            let outer = comp(vec![consumer, matte_layer]);
            let outer_id = outer.id;
            doc.items.push(ProjectItem::Composition(outer));

            let probes: HashMap<Uuid, crate::SourceProbe> = [(
                item,
                crate::SourceProbe::Video {
                    fps: 60.0,
                    width: 64,
                    height: 64,
                    frames: 600,
                    audio: false,
                },
            )]
            .into_iter()
            .collect();
            let outer = doc.comp(outer_id).unwrap();
            let jobs = plan_comp_frame(&doc, outer, 0.0, Quality::default(), &probes);
            let how = if track_matte {
                "a track matte"
            } else {
                "a layer-input matte"
            };
            assert_eq!(
                jobs.len(),
                1,
                "{how} onto a precomp must plan the one decode its footage needs"
            );
            assert_eq!(jobs[0].item, item);
        }
    }

    /// **A held nested frame wants no decodes** (K-422). A Precomp whose frame
    /// the store already holds contributes none of its footage jobs — that is
    /// what makes a parent edit free of the nested comp's decodes — while a
    /// collapsed Precomp (spliced in, never named) still decodes whatever the
    /// answerer says. Fails without the `held` question in the Precomp arm.
    #[test]
    fn a_held_nested_frame_plans_no_decodes_but_a_collapsed_one_still_does() {
        use lumit_core::model::{
            Composition, Document, FootageItem, Layer, LayerKind, LinearColour, MediaRef, Switches,
            TransformGroup,
        };
        use lumit_core::time::{CompTime, Duration, FrameRate, Rational};
        use std::collections::HashMap;

        let layer = |kind: LayerKind| Layer {
            markers: Vec::new(),
            id: Uuid::now_v7(),
            name: "l".into(),
            kind,
            in_point: CompTime(Rational::ZERO),
            out_point: CompTime(Rational::new(10, 1).unwrap()),
            start_offset: CompTime(Rational::ZERO),
            transform: TransformGroup::default(),
            matte: None,
            parent: None,
            label: 0,
            volume_db: lumit_core::anim::Property::zero(),
            retime: None,
            interpolation: lumit_core::retime::Interpolation::default(),
            parked_flow: None,
            blend: lumit_core::model::BlendMode::default(),
            masks: Vec::new(),
            paint: Vec::new(),
            effects: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        };
        let comp = |layers: Vec<Layer>| Composition {
            id: Uuid::now_v7(),
            name: "c".into(),
            width: 64,
            height: 64,
            frame_rate: FrameRate::new(60, 1).unwrap(),
            duration: Duration(Rational::new(10, 1).unwrap()),
            background: LinearColour::BLACK,
            work_area: None,
            layers,
            markers: Vec::new(),
            motion_blur: lumit_core::model::MotionBlur::default(),
            extra: serde_json::Map::new(),
        };
        let mut doc = Document::new();
        let item = Uuid::now_v7();
        doc.items.push(ProjectItem::Footage(FootageItem {
            id: item,
            name: "f".into(),
            media: MediaRef {
                relative_path: "f.mp4".into(),
                absolute_path: "/f.mp4".into(),
                fingerprint: None,
                extra: serde_json::Map::new(),
            },
            extra: serde_json::Map::new(),
        }));
        let inner = comp(vec![layer(LayerKind::Footage { item })]);
        let inner_id = inner.id;
        doc.items.push(ProjectItem::Composition(inner));
        let outer = comp(vec![layer(LayerKind::Precomp { comp: inner_id })]);
        let outer_id = outer.id;
        doc.items.push(ProjectItem::Composition(outer));
        let probes: HashMap<Uuid, crate::SourceProbe> = [(
            item,
            crate::SourceProbe::Video {
                fps: 60.0,
                width: 64,
                height: 64,
                frames: 600,
                audio: false,
            },
        )]
        .into_iter()
        .collect();
        let asked = std::cell::Cell::new(0usize);
        let held = |nested: &Composition, lt: f64| {
            asked.set(asked.get() + 1);
            assert_eq!(nested.id, inner_id);
            assert_eq!(lt, 0.0);
            true
        };
        let plan = |doc: &Document, held: Option<HeldNested<'_>>| {
            plan_comp_frame_held(
                doc,
                doc.comp(outer_id).unwrap(),
                0.0,
                Quality::default(),
                &probes,
                held,
            )
        };
        assert_eq!(
            plan(&doc, None).len(),
            1,
            "no answerer: the footage decodes"
        );
        assert!(
            plan(&doc, Some(&held)).is_empty(),
            "held: nothing to decode"
        );
        assert_eq!(asked.get(), 1);

        doc.comp_mut(outer_id).unwrap().layers[0].switches.collapse = true;
        assert_eq!(
            plan(&doc, Some(&held)).len(),
            1,
            "a collapsed precomp is never named, so it always decodes"
        );
        assert_eq!(asked.get(), 1, "and is never asked about");
    }
}

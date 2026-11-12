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

use crate::decode::{CompJob, Cut};
use crate::overlap::under_incoming;
use crate::source::SourceProbes;
use lumit_core::fx::temporal::FlowWant;
use lumit_core::model::{Composition, Document, LayerKind};
use std::path::PathBuf;
use uuid::Uuid;
mod synthesis_key;

/// While the user is actively scrubbing or dragging, footage decodes at most
/// this wide so a frame comes back fast (the specified resolution reloads the
/// moment they stop). Chosen to keep even 4K sources instant to draft.
pub const DRAFT_MAX_WIDTH: u32 = 640;

/// A decode width that also fits the largest picture. `target` is what the
/// quality policy asked for (`None` for native); the answer is the same
/// unless the source at that width would still be wider or taller than
/// [`lumit_core::largest_picture::side`], in which case it is the widest that
/// fits, aspect kept. Applied after the policy, and to the native path too, so
/// no footage is ever asked for at a size the card cannot hold: a 9 000-pixel
/// photograph uploaded at its own size is a validation error on every pass
/// that touches it, and the layer comes out blank. Shrunk to fit, it is a
/// picture.
#[must_use]
pub fn fit_texture(target: Option<u32>, natural_w: u32, natural_h: u32) -> Option<u32> {
    let most = lumit_core::largest_picture::side();
    let side = natural_w.max(natural_h);
    if side <= most {
        return target;
    }
    let widest = (u64::from(natural_w) * u64::from(most) / u64::from(side)) as u32;
    if target.is_none_or(|t| t > widest) {
        lumit_core::largest_picture::note_cut();
    }
    Some(target.map_or(widest, |t| t.min(widest)).max(16))
}

/// How coarsely to decode this preview — the quality axis of both the decode
/// plan and the frame-cache key. Keeping the two in one type is deliberate: if
/// they could disagree, a frame decoded at one width could be served from a
/// cache entry filed under another.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
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

/// The nested-frame question a plan asks ([`PlanContext::held`]):
/// "is this comp's frame at this layer time already a finished texture?"
///
/// The instance is a placed node graph's own Input values
/// (docs/impl/node-graph-comp.md §5.3), which are part of the frame's name, so
/// what the builder names and what the planner skips agree. `None` everywhere
/// else, which is every layer over a layer comp.
///
/// The document is the one the comp is read from, which for a copy of a
/// graphics template is the one its values make.
pub type HeldNested<'a> = &'a dyn Fn(
    &std::sync::Arc<Document>,
    &Composition,
    f64,
    Option<&lumit_core::model::EffectInstance>,
) -> bool;

/// The inputs a plan walk carries down the Precomp recursion, unchanged at
/// every depth: what the media is and how coarsely to decode. Bundled so the
/// recursive walk stays readable.
pub struct PlanContext<'a> {
    pub doc: &'a Document,
    /// `doc` as the renderer shares it, when it does. A copy of a graphics
    /// template is planned in the document its values make, and making one
    /// starts from this. `None` plans every copy as the template itself.
    pub shared: Option<&'a std::sync::Arc<Document>>,
    pub quality: Quality,
    pub probes: &'a dyn SourceProbes,
    /// Answers "is this nested comp's frame, at this layer time, already held
    /// as a finished texture?". When it is, the planner asks for none
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

/// The decode source for one footage item: the file its pixels are read from
/// (already resolved through the proxy rule) and — for a numbered run of
/// stills — the rate to read the run at.
///
/// The rate rides only when the file *is* the item's own media. A proxy is one
/// file standing in for the whole run, so reading it as a sequence would be
/// reading files that are not there. The same goes for a source layer or a
/// render pass: a proxy has no layers or passes to pick from.
fn media_source(
    doc: &Document,
    item: Uuid,
    media: &lumit_core::model::MediaRef,
) -> lumit_media::sequence::MediaSource {
    let footage = match doc.item(item) {
        Some(lumit_core::model::ProjectItem::Footage(f)) => Some(f),
        _ => None,
    };
    let (sequence_fps, source_layer, render_pass) = match footage {
        Some(f) if std::ptr::eq(&f.media, media) => (
            f.sequence_fps(),
            f.source_layer,
            f.interpretation.render_pass.clone(),
        ),
        _ => (None, None, None),
    };
    lumit_media::sequence::MediaSource {
        path: PathBuf::from(&media.absolute_path),
        sequence_fps,
        source_layer,
        render_pass,
        // A proxy is the same frames, so it is read the way its original is.
        interpret: footage
            .map(crate::source::interpretation)
            .unwrap_or_default(),
        stamp: None,
    }
    .stamped()
}

/// The neighbour frames a Precomp or adjustment layer's stack reads at layer
/// time `lt`, each with whether a flow consumer wants motion measured against
/// it. These layers have no decoded frames, so their picture is built again at
/// each offset. The planner and the builder both ask here, so the footage one
/// fetches is the footage the other draws. Empty for a stack that reads only
/// its own frame.
///
/// The second half of each entry is the flow offset a field measured against
/// that picture is filed under and how it is wanted, `None` when nothing
/// measures it. On the frame before a cut (`crosses`, asked per offset) Motion
/// blur is measured a frame back and filed under its own `1`, so the next shot
/// is never read as motion.
pub(crate) fn rebuild_offsets(
    layer: &lumit_core::model::Layer,
    lt: f64,
    comp_dt: f64,
    crosses: impl Fn(i32) -> bool,
) -> Vec<(i32, Option<(i32, FlowWant)>)> {
    let fx_on = layer.switches.fx;
    let flow = lumit_core::fx::temporal::stack_flow_sides_wanted(&layer.effects, fx_on, crosses);
    let mut offsets: Vec<i32> = flow.iter().map(|&(_, side, _)| side).collect();
    if lumit_core::fx::temporal::stack_is_temporal(&layer.effects, fx_on) {
        // Asked at the layer's frame, as a footage layer's window is.
        offsets.extend(lumit_core::fx::temporal::stack_temporal_window(
            &layer.effects,
            fx_on,
            lt / comp_dt,
            1.0 / comp_dt,
        ));
    }
    offsets.retain(|&o| o != 0);
    offsets.sort_unstable();
    offsets.dedup();
    // ponytail: one field per picture. Datamosh and Motion blur on one layer
    // both measuring a frame back lose Motion blur on that frame.
    offsets
        .into_iter()
        .map(|o| {
            let measure = flow.iter().find(|&&(_, side, _)| side == o);
            (o, measure.map(|&(f, _, want)| (f, want)))
        })
        .collect()
}

/// How far comp time `tau` is from the frame time `t`, in comp frames: the
/// key a clip's picture at another moment is filed under and found by
/// ([`CompLayerPixels::shutter`](crate::decode::CompLayerPixels::shutter)).
pub(crate) fn moment_offset(tau: f64, t: f64, comp_dt: f64) -> f64 {
    (tau - t) / comp_dt
}

/// The other moments a Clone to points shows its clone layers at, as the
/// layer and the comp time. It is the list the builder renders, asked of the
/// same code, so the footage fetched is the footage drawn. `lt` is the time of
/// the layer the effect is on. Empty for any other effect, and with Time
/// offset off.
fn clone_moments_of(
    e: &lumit_core::model::EffectInstance,
    doc: &Document,
    comp: &Composition,
    t: f64,
    lt: f64,
) -> Vec<(Uuid, f64)> {
    use lumit_core::fx::effects::clone_to_points::CloneToPoints;
    // Time loop's Source layer, at the moments the loop shows.
    if e.effect.match_name == "time_loop" {
        return lumit_core::fx::effects::time_loop::TimeLoop::planned(e, comp, t, lt)
            .into_iter()
            .filter(|(_, at)| at.to_bits() != t.to_bits())
            .collect();
    }
    if e.effect.match_name != "clone_to_points" {
        return Vec::new();
    }
    CloneToPoints::planned(e, doc, comp, t, lt, None)
        .into_iter()
        .flatten()
        .filter(|p| p.time.to_bits() != t.to_bits())
        .map(|p| (p.layer, p.time))
        .collect()
}

/// File a clip's picture at each of `moments` on its job, so a neighbour of
/// a Precomp or an adjustment layer above shows the footage a frame away and
/// not this one. One real frame each, picked the way a layer's own
/// neighbours are. `source_at` is the source time the clip shows at a comp
/// time.
fn push_moments(
    shutter: &mut Vec<crate::decode::ShutterSample>,
    moments: &[f64],
    t: f64,
    comp_dt: f64,
    fps: f64,
    src_frames: usize,
    source_at: impl Fn(f64) -> f64,
) {
    for &tau in moments {
        let offset = moment_offset(tau, t, comp_dt);
        if offset == 0.0
            || shutter
                .iter()
                .any(|s| s.offset.to_bits() == offset.to_bits())
        {
            continue;
        }
        let (source_frame, _) =
            lumit_core::pixels::frame_pick(source_at(tau), fps, src_frames, false, None);
        shutter.push(crate::decode::ShutterSample {
            offset,
            source_frame,
            blend: None,
        });
    }
}

/// Recursively collect the decode jobs comp `comp` needs at comp time `t`
/// (docs/06-RENDER-PIPELINE.md: Precomp evaluation). Cycle-guarded through
/// `visited`, which must already contain `comp.id`.
///
/// The context's `probes` answers what each footage item is; an unprobed item
/// contributes no job at all and is retried once its probe lands.
///
/// `spliced` says this comp is being spliced into its parent by a collapsed
/// Precomp layer, where the occlusion cull does not apply — the same
/// flag the draw builder takes, so the two skip exactly the same layers.
///
/// `moments` are the other times of this comp a temporal effect further up
/// builds it again at ([`rebuild_offsets`]), so its footage is fetched at
/// those too. Empty for every comp nothing temporal looks into.
pub fn collect_comp_jobs(
    ctx: &PlanContext<'_>,
    comp: &Composition,
    t: f64,
    moments: &[f64],
    jobs: &mut Vec<CompJob>,
    visited: &mut Vec<Uuid>,
    spliced: bool,
) {
    let PlanContext {
        doc,
        shared,
        quality,
        probes,
        held,
    } = *ctx;
    // **A node graph composition** (docs/impl/node-graph-comp.md §2.1). It has
    // no layers, so the walk below would find nothing: what it has is Read
    // boxes, each the layer it behaves like, and each planned by that layer.
    if let Some(graph) = &comp.graph {
        // Viewed on its own or placed as a Precomp layer, so a picture Input's
        // preview item is drawn and has to be decoded (§5.11).
        collect_graph_jobs(ctx, comp, graph, t, moments, jobs, visited, false);
        return;
    }
    let in_span =
        |l: &lumit_core::model::Layer| t >= l.in_point.0.to_f64() && t < l.out_point.0.to_f64();
    // Occlusion cull (docs/06 §1.1): the layers under a full-frame
    // opaque layer are never seen, so their footage is never decoded. The
    // predicate refuses whenever anything above could reach a layer below,
    // so nothing a wanted layer references is ever culled.
    let occluder = (!spliced)
        .then(|| lumit_core::occlusion::occluder_index(doc, comp, t))
        .flatten();
    // Solo / isolate: while a layer that *draws* is soloed, only
    // soloed layers reach the picture — so only they are worth decoding. The
    // same question the draw builder, the frame key and the occlusion cull all
    // ask, asked here too, on the rule the occlusion comment above states: a
    // layer that cannot be seen is a layer that is never decoded.
    //
    // Without it a comp with one soloed row still decoded every other visible
    // row behind it. On a sixty-four-layer edit at full resolution that is
    // dozens of simultaneous 4K decodes feeding a composite that draws one
    // layer, which is the shape of a session that ends without a message.
    let any_solo = lumit_core::model::any_picture_solo(comp);
    let mut wanted: Vec<Uuid> = comp.environment_source(t).into_iter().collect();
    // Whether a layer is drawn, which is the walk below's own gate.
    let drawn = |idx: usize, l: &lumit_core::model::Layer| {
        !occluder.is_some_and(|o| idx > o)
            && !l.audio_only
            && l.switches.visible
            && in_span(l)
            && !(any_solo && !l.switches.solo)
    };
    // The other moments a Clone to points or a held projection shows a layer
    // at, as the layer and the comp time. It is the list the builder renders,
    // asked of the same code, so its footage is fetched at every one of them.
    // Gathered before the walk, which plans the graphs a layer applies too.
    let clone_moments: Vec<(Uuid, f64)> = comp
        .layers
        .iter()
        .enumerate()
        .filter(|(idx, l)| drawn(*idx, l))
        .flat_map(|(_, l)| {
            let lt = lumit_core::time::layer_time(t, l.start_offset.0);
            l.effects
                .iter()
                .filter(|e| e.enabled)
                .flat_map(move |e| clone_moments_of(e, doc, comp, t, lt))
                .chain(lumit_core::model_material::held_moments(l, comp, lt))
        })
        .collect();
    let comp_dt = 1.0 / comp.frame_rate.fps().max(1.0);
    // The times each layer is built again at: the ones this comp was handed,
    // and for a layer under an adjustment with a temporal stack, that
    // adjustment's neighbours. The builder rebuilds the layers below at the
    // same times (`adjustment_flow_below`). Empty everywhere in an ordinary
    // comp.
    let mut rebuilt = moments.to_vec();
    let mut layer_moments = Vec::with_capacity(comp.layers.len());
    for (idx, l) in comp.layers.iter().enumerate() {
        // And the moments a Clone to points shows this layer at, and the ones
        // an effect on it shows its own input at, which on an adjustment
        // layer are the layers' below.
        let cloned = clone_moments.iter().filter(|(id, _)| *id == l.id);
        // At the frame and at each moment an adjustment above builds this
        // layer again at, as that rebuild draws the layer's own moments too.
        let own: Vec<f64> = if l.switches.visible && !l.audio_only {
            std::iter::once(t)
                .chain(rebuilt.iter().copied())
                .flat_map(|tau| lumit_core::fx::moments::own_moment_comp_times(comp, l, tau))
                .collect()
        } else {
            Vec::new()
        };
        layer_moments.push(
            rebuilt
                .iter()
                .copied()
                .chain(cloned.map(|(_, tau)| *tau))
                .chain(own.iter().copied().filter(|_| !l.is_adjustment()))
                .collect::<Vec<f64>>(),
        );
        if l.is_adjustment() {
            rebuilt.extend(own);
        }
        if l.is_adjustment()
            && l.switches.visible
            && !l.graph.out_unwired
            && in_span(l)
            && !(any_solo && !l.switches.solo)
        {
            let lt = lumit_core::time::layer_time(t, l.start_offset.0);
            let below = &comp.layers[idx + 1..];
            let crosses = |o| {
                lumit_core::fx::temporal::cut_between(
                    below,
                    t,
                    lumit_core::time::frames_on(t, o, comp_dt),
                )
            };
            rebuilt.extend(
                rebuild_offsets(l, lt, comp_dt, crosses)
                    .into_iter()
                    .map(|(o, _)| lumit_core::time::frames_on(t, o, comp_dt)),
            );
        }
    }
    // A layer out of its span now may be in it at one of those moments: a
    // clone at a moment a stamp shows, or the next shot on the frame before a
    // cut, which is the neighbour a temporal effect above the two reads. It
    // is planned all the same, or that neighbour is drawn without it.
    let moment_in_span = |idx: usize, l: &lumit_core::model::Layer| {
        layer_moments.get(idx).is_some_and(|moments| {
            moments
                .iter()
                .any(|tau| *tau >= l.in_point.0.to_f64() && *tau < l.out_point.0.to_f64())
        })
    };
    // Those moments on a layer's own clock, which is the clock a graph it
    // applies runs on.
    let graph_moments = |l: &lumit_core::model::Layer| -> Vec<f64> {
        clone_moments
            .iter()
            .filter(|(id, _)| *id == l.id)
            .map(|(_, tau)| lumit_core::time::layer_time(*tau, l.start_offset.0))
            .collect()
    };
    for (idx, l) in comp.layers.iter().enumerate() {
        if occluder.is_some_and(|o| idx > o) {
            continue;
        }
        // An Audio layer decodes for the mixer, never for the picture.
        if l.audio_only {
            continue;
        }
        // As the build draws it, so a layer a clip's effect reads is decoded.
        let worn =
            lumit_core::sequence::worn_at(l, lumit_core::time::layer_time(t, l.start_offset.0));
        let l = worn.as_ref().unwrap_or(l);
        if l.switches.visible
            && (in_span(l) || moment_in_span(idx, l))
            && !(any_solo && !l.switches.solo)
        {
            // A layer acting as an adjustment draws the composite
            // beneath it, so its OWN frames are never asked for — decoding them
            // would be a video decode nobody looks at. Only its own frames,
            // though: it still gates by a matte and still feeds its effects'
            // layer inputs, and skipping the whole layer here took those
            // references with it. A matte source is normally hidden, so it was
            // wanted by nothing else and never decoded — which is a matte that
            // works while its source layer is switched on and stops the moment
            // it is switched off.
            if !l.is_adjustment() {
                wanted.push(l.id);
            }
            if let Some(m) = &l.matte {
                if !wanted.contains(&m.layer) {
                    wanted.push(m.layer);
                }
            }
            // Layer-input references (e.g. a DoF depth pass) decode
            // exactly like matte sources: the referenced layer is usually
            // hidden (you don't want the depth map rendering), but its
            // pixels still feed the effect, or a model's material.
            for e in lumit_core::model_material::and_effects(l).filter(|e| e.enabled) {
                for p in &e.params {
                    if let lumit_core::model::EffectValue::Layer(Some(id)) = p.value {
                        if !wanted.contains(&id) {
                            wanted.push(id);
                        }
                    }
                }
                // **A Node graph effect** (docs/impl/node-graph-comp.md §2.4)
                // brings a whole comp's footage in through this layer's stack,
                // so the graph it names is planned under the guard a Precomp
                // layer's comp is planned under. Here rather than in the walk
                // below because an adjustment layer never reaches that walk -
                // its own frames are deliberately not decoded - and a graph on
                // one still has footage to decode.
                //
                // At the time this layer's own ops resolve at, so a Posterize
                // time on the layer holds the graph's decodes with them (§5.2).
                let lt = lumit_core::fx::temporal::this_layer_effect_time(
                    &l.effects,
                    l.switches.fx,
                    lumit_core::time::layer_time(t, l.start_offset.0),
                    l.start_offset.0,
                );
                nested_graph_jobs(ctx, e, lt, &graph_moments(l), jobs, visited);
            }
        }
    }
    // A layer read only as another effect's picture or as a matte is not
    // drawn, so the walk above passed over its stack. Its Node graph effects
    // still run when it is read, and the footage their graphs read is planned
    // here, at the moments a stamp shows the layer as well.
    for (idx, l) in comp.layers.iter().enumerate() {
        if drawn(idx, l)
            || !wanted.contains(&l.id)
            || !(in_span(l) || moment_in_span(idx, l))
            || !l.switches.fx
        {
            continue;
        }
        let lt = lumit_core::time::layer_time(t, l.start_offset.0);
        for e in l.effects.iter().filter(|e| e.enabled) {
            nested_graph_jobs(ctx, e, lt, &graph_moments(l), jobs, visited);
        }
    }
    // **And the same effect on a live group's header** (docs/impl/
    // group-effects.md §2): the header's stack runs on the members' composite,
    // so a graph on it reads footage the layer walk above never sees. The
    // group's clock is the comp's, since a header carries no start offset.
    for group in comp
        .groups
        .iter()
        .filter(|g| lumit_core::group::header_live(g))
    {
        for e in &group.effects {
            nested_graph_jobs(ctx, e, t, &[], jobs, visited);
        }
    }
    // Posterize Time (docs/08 §3.25, FX-1): a layer covered by a live
    // Posterize decodes its source at the held grid time, the decode twin of
    // the held re-render the draw builder performs, and one only a held
    // projection reads at the frame it holds. `sample_times[idx]` is the comp
    // time for `comp.layers[idx]`, `t` for every layer in an ordinary comp.
    let sample_times = lumit_core::fx::temporal::posterize_sample_times(&comp.layers, t);
    let sample_times = lumit_core::model_material::with_held_frames(comp, sample_times, t);
    // Accumulation motion blur (docs/08 §3.26): the sub-frame moments each
    // layer's footage is wanted at by the adjustments above it, in comp
    // frames. Empty everywhere in an ordinary comp.
    let shutter_offsets = lumit_core::fx::temporal::accumulation_shutter_offsets(&comp.layers, t);
    for (idx, layer) in comp.layers.iter().enumerate() {
        if !wanted.contains(&layer.id) || !(in_span(layer) || moment_in_span(idx, layer)) {
            continue;
        }
        let lt = lumit_core::time::layer_time(sample_times[idx], layer.start_offset.0);
        // As the build draws it, so a clip's temporal effect is planned its
        // neighbours.
        let worn = lumit_core::sequence::worn_at(layer, lt);
        let layer = worn.as_ref().unwrap_or(layer);
        match &layer.kind {
            // No footage source to decode (an adjustment layer processes
            // the composite below; solids/text/cameras rasterise elsewhere).
            LayerKind::Solid { .. }
            | LayerKind::Text { .. }
            | LayerKind::Shape { .. }
            | LayerKind::Camera { .. }
            | LayerKind::Light { .. }
            | LayerKind::Model { .. }
            | LayerKind::Mesh { .. }
            | LayerKind::Adjustment
            | LayerKind::Null => {}
            LayerKind::Sequence { clips } => {
                let flow_neighbours =
                    lumit_core::fx::temporal::stack_flow_wants(&layer.effects, layer.switches.fx);
                use lumit_core::sequence::{Clip, ClipSource, Shown};
                // The job for one clip at layer time `lt`, alone (a clip
                // whose source is a comp has no job). `whole` asks for
                // one real frame of it, the way a neighbour is picked: no
                // blend partner and no flow.
                let footage_job = |clip: &Clip, lt: f64, whole: bool| -> Option<CompJob> {
                    let ClipSource::Footage(item) = clip.source else {
                        return None;
                    };
                    let st = clip.held_source_time(lt);
                    // The one proxy resolution point: which file this
                    // item's pixels come from, and the probe to believe about
                    // them (always the original's — see `effective_media`).
                    let (media, probe) = crate::source::effective_media(doc, probes, item)?;
                    let (fps, nat_w, nat_h, src_frames) = probe.video()?;
                    use lumit_core::retime::Interpolation;
                    let clip = (!whole).then_some(clip);
                    // Same engagement gate as a Footage layer; the
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
                    // Flow decodes natively, and so does a layer whose stack
                    // measures motion, as a Footage layer's does.
                    let target_width = if flow.is_some() || !flow_neighbours.is_empty() {
                        None
                    } else {
                        quality.target_width(nat_w)
                    };
                    let target_width = fit_texture(target_width, nat_w, nat_h);
                    let (source_frame, blend) =
                        lumit_core::pixels::frame_pick(st, fps, src_frames, blend_on, sample_fps);
                    Some(CompJob {
                        layer: layer.id,
                        at: t,
                        item,
                        source: media_source(doc, item, media),
                        source_frame,
                        target_width,
                        natural_w: nat_w,
                        natural_h: nat_h,
                        blend,
                        flow,
                        temporal: Vec::new(),
                        flow_neighbours: Vec::new(),
                        slate: false,
                        channels: lumit_core::fx::temporal::stack_extracted_channels(
                            &layer.effects,
                            layer.switches.fx,
                        ),
                        // The accumulation shutter is a later refinement: a
                        // clip under one is held.
                        shutter: Vec::new(),
                        shutter_flow: None,
                        cuts: Vec::new(),
                    })
                };
                let plays_comp = |clip: &Clip| matches!(clip.source, ClipSource::Comp(_));
                // The job for what the layer shows at layer time `lt`
                // (`lumit_core::sequence::shown_at`). One whole clip is that
                // clip's job and nothing more. Inside a fade the job carries
                // its gain, and inside a dissolve the incoming clip's job
                // carries the gain and the outgoing clip's job under it.
                let shown_job = |shown: Shown<'_>, lt: f64, whole: bool| -> Option<CompJob> {
                    match shown {
                        Shown::Nothing => None,
                        Shown::One { clip, gain } => {
                            let mut job = footage_job(clip, lt, whole)?;
                            if gain < 1.0 {
                                job.cuts.push(Cut::Under(gain, None));
                            }
                            Some(job)
                        }
                        // A composition has no decoded picture to mix here,
                        // so a dissolve with one on either side is mixed
                        // where it is drawn, and the footage clip beside it
                        // is a plain job.
                        Shown::Two {
                            outgoing, incoming, ..
                        } if plays_comp(outgoing) || plays_comp(incoming) => {
                            footage_job(incoming, lt, whole)
                                .or_else(|| footage_job(outgoing, lt, whole))
                        }
                        Shown::Two {
                            outgoing,
                            incoming,
                            gain,
                            ..
                        } => {
                            let under = footage_job(outgoing, lt, whole);
                            // An incoming clip with no picture of its own
                            // leaves the outgoing one as it is.
                            let Some(mut job) = footage_job(incoming, lt, whole) else {
                                return under;
                            };
                            job.cuts.push(under_incoming(incoming, whole, gain, under));
                            Some(job)
                        }
                    }
                };
                let clip_job = |lt: f64, whole: bool| {
                    shown_job(lumit_core::sequence::shown_at(clips, lt), lt, whole)
                };
                let shown = lumit_core::sequence::shown_at(clips, lt);
                // A clip that plays a composition is planned as a Precomp
                // layer's comp is: the footage inside it, at the moment the
                // clip shows, behind the same guard against a comp inside
                // itself. A frame of it that is already a finished texture
                // wants no decodes, asked where the builder asks by the same
                // name, which is at the live time.
                //
                // ponytail: at the frame's own time only. A temporal effect
                // that reads this layer at another moment is not handed a
                // composition clip's footage as it is then. The upgrade is
                // the other moments a Precomp layer plans.
                let (first, second) = match shown {
                    Shown::Nothing => (None, None),
                    Shown::One { clip, .. } => (Some(clip), None),
                    Shown::Two {
                        outgoing, incoming, ..
                    } => (Some(outgoing), Some(incoming)),
                };
                for clip in first.into_iter().chain(second) {
                    let ClipSource::Comp(nested_id) = clip.source else {
                        continue;
                    };
                    // A clip that is a copy of a graphics template is planned
                    // in the document its values make, so a picture it puts in
                    // the template is the one fetched.
                    let placed = shared.and_then(|shared| {
                        lumit_core::template::placed::for_clip(
                            shared,
                            comp,
                            layer,
                            clip,
                            lt,
                            sample_times[idx],
                        )
                    });
                    let within = placed.as_ref().map(|placed| PlanContext {
                        doc: placed,
                        shared: Some(placed),
                        quality,
                        probes,
                        held,
                    });
                    let (ctx, doc) = match (&within, &placed) {
                        (Some(within), Some(placed)) => (within, &**placed),
                        _ => (ctx, doc),
                    };
                    // A multicam clip decodes its angle and no other.
                    let Some(nested) = lumit_core::multicam::nested_for(doc, clip)
                        .filter(|_| !visited.contains(&nested_id))
                    else {
                        continue;
                    };
                    let nested = nested.as_ref();
                    let st = clip.nested_time(nested, lt);
                    if sample_times[idx] == t
                        && held
                            .zip(placed.as_ref().or(shared))
                            .is_some_and(|(held, read)| held(read, nested, st, None))
                    {
                        continue;
                    }
                    visited.push(nested_id);
                    collect_comp_jobs(ctx, nested, st, &[], jobs, visited, false);
                    visited.pop();
                }
                let live = shown_job(shown, lt, false);
                let in_gap = matches!(shown, Shown::Nothing);
                // A layer in a gap now may show a clip at a moment a temporal
                // effect above builds it again at, and a job is the only
                // thing a clip's pixels travel in. So the first such clip
                // stands as the job, and the frame's own moment is filed as
                // the gap it is, which is how the builder knows to draw
                // nothing here.
                let stand_in = || {
                    // A Posterize-held layer keeps what it holds.
                    let held = sample_times[idx] != t;
                    let mut moments = layer_moments[idx].iter().filter(|_| in_gap && !held);
                    moments.find_map(|&tau| {
                        clip_job(
                            lumit_core::time::layer_time(tau, layer.start_offset.0),
                            true,
                        )
                    })
                };
                let Some(mut job) = live.or_else(stand_in) else {
                    continue;
                };
                if in_gap {
                    job.cuts.push(Cut::Moment(0.0, None));
                }
                // Neighbour frames for a temporal effect stack, as a Footage
                // layer's are, through whichever clip is live then. A clip of
                // this job's footage is one more frame of it, and any other
                // is a job of its own, as is one inside a fade or a dissolve.
                // A gap is no neighbour, and for Denoise nor is another clip.
                let other =
                    |at| lumit_core::fx::temporal::denoise_cut(&flow_neighbours, clips, lt, at);
                if !in_gap
                    && lumit_core::fx::temporal::stack_is_temporal(
                        &layer.effects,
                        layer.switches.fx,
                    )
                {
                    let window = lumit_core::fx::temporal::stack_temporal_window(
                        &layer.effects,
                        layer.switches.fx,
                        lt / comp_dt,
                        1.0 / comp_dt,
                    );
                    for o in window.into_iter().filter(|&o| o != 0) {
                        let at = lumit_core::time::frames_on(lt, o, comp_dt);
                        match clip_job(at, true) {
                            Some(n) if n.item == job.item && n.cuts.is_empty() && !other(at) => {
                                job.temporal.push((o, n.source_frame));
                            }
                            Some(n) => job.cuts.push(Cut::Neighbour(o, n)),
                            None => {}
                        }
                    }
                    job.flow_neighbours = flow_neighbours.clone();
                }
                // And the layer at each moment a temporal effect above builds
                // it again at: the clip live then, or nothing in a gap. A
                // Posterize-held clip stays held.
                if sample_times[idx] == t {
                    for &tau in &layer_moments[idx] {
                        let offset = moment_offset(tau, t, comp_dt);
                        let at = offset.to_bits();
                        let filed = |c: &Cut| matches!(c, Cut::Moment(o, _) if o.to_bits() == at);
                        if offset != 0.0 && !job.cuts.iter().any(filed) {
                            let lt = lumit_core::time::layer_time(tau, layer.start_offset.0);
                            job.cuts.push(Cut::Moment(offset, clip_job(lt, true)));
                        }
                    }
                }
                jobs.push(job);
            }
            LayerKind::Precomp { comp: nested_id } => {
                if visited.contains(nested_id) {
                    continue; // cycle guard
                }
                if let Some(nested) = doc.comp(*nested_id) {
                    // A copy of a graphics template is planned in the document
                    // its values make, so a picture it puts in the template is
                    // the one fetched.
                    let placed = shared.and_then(|shared| {
                        lumit_core::template::placed::for_layer(
                            shared,
                            comp,
                            layer,
                            lt,
                            sample_times[idx],
                            None,
                        )
                    });
                    let within = placed.as_ref().map(|placed| PlanContext {
                        doc: placed,
                        shared: Some(placed),
                        quality,
                        probes,
                        held,
                    });
                    let (ctx, doc, nested) = match (&within, &placed) {
                        (Some(within), Some(placed)) => {
                            (within, &**placed, placed.comp(*nested_id).unwrap_or(nested))
                        }
                        _ => (ctx, doc, nested),
                    };
                    // The Retime map (docs/impl/node-graph-comp.md §5.6): the
                    // moment of the nested comp this layer shows, which is the
                    // moment the builder evaluates and names. Equal to `lt`
                    // for a layer with no map.
                    let mut st = lumit_core::model::nested_source_time(layer, nested, lt);
                    // A held nested frame wants no decodes. Asked only
                    // where the builder will ask by the same name: at the live
                    // time (a Posterize-held layer is built at another time,
                    // by a walk with no keyer) and for a Precomp that is not
                    // collapsed (a collapsed one is spliced in, never named).
                    let live = sample_times[idx] == t;
                    // On Flow between two frames, the frame before is built
                    // and the frame after is one more moment of it.
                    let pair = live
                        .then(|| {
                            lumit_core::precomp_flow::pair(layer, nested, lt, comp.frame_rate.fps())
                        })
                        .flatten();
                    if let Some((a, _, _)) = pair {
                        st = a;
                    }
                    let collapsed = matches!(
                        lumit_core::model::collapse_state(doc, comp, layer, lt),
                        lumit_core::model::CollapseState::Active
                    );
                    // The other moments of the nested comp the builder asks
                    // for: the ones this layer is itself rebuilt at, and its
                    // own stack's neighbours. Each goes through the Retime
                    // map the way `st` did. A Posterize-held layer keeps its
                    // held footage.
                    let mut nested_moments = Vec::new();
                    if live {
                        let nested_at = |tau: f64| {
                            lumit_core::model::nested_source_time(
                                layer,
                                nested,
                                lumit_core::time::layer_time(tau, layer.start_offset.0),
                            )
                        };
                        nested_moments.extend(layer_moments[idx].iter().map(|&tau| nested_at(tau)));
                        if !collapsed && !layer.is_adjustment() {
                            let crosses = |o| {
                                lumit_core::fx::temporal::cut_between(
                                    &nested.layers,
                                    st,
                                    nested_at(t + f64::from(o) * comp_dt),
                                )
                            };
                            nested_moments.extend(
                                rebuild_offsets(layer, lt, comp_dt, crosses)
                                    .into_iter()
                                    .map(|(o, _)| nested_at(t + f64::from(o) * comp_dt)),
                            );
                        }
                    }
                    nested_moments.extend(pair.map(|(_, b, _)| b));
                    // A rebuild at another moment is made from pixels, so a
                    // held frame only saves the decodes when there is none.
                    if live
                        && !collapsed
                        && nested_moments.is_empty()
                        && held
                            .zip(placed.as_ref().or(shared))
                            .is_some_and(|(held, read)| {
                                // A template's copy is named by its document
                                // alone, as the builder names it.
                                let inputs = layer.graph_inputs.as_ref().filter(|i| {
                                    lumit_core::fx::effects::template::comp_of(i).is_none()
                                });
                                held(read, nested, st, inputs)
                            })
                    {
                        continue;
                    }
                    visited.push(*nested_id);
                    collect_comp_jobs(ctx, nested, st, &nested_moments, jobs, visited, collapsed);
                    visited.pop();
                }
            }
            LayerKind::Footage { item } => {
                // The one proxy resolution point: which file this
                // item's pixels come from, and the probe to believe about them
                // (always the original's — see `effective_media`).
                let Some((media, probe)) = crate::source::effective_media(doc, probes, *item)
                else {
                    continue;
                };
                // Missing media still draws (docs/07 §3.3): a slate job at
                // comp size, so the layer shows test bars in place of the
                // picture instead of silently vanishing. Sized to the comp
                // because a file we cannot open has no size to report.
                if probe.slates() {
                    jobs.push(CompJob {
                        layer: layer.id,
                        at: t,
                        item: *item,
                        source: media_source(doc, *item, media),
                        source_frame: 0,
                        target_width: None,
                        natural_w: comp.width,
                        natural_h: comp.height,
                        blend: None,
                        flow: None,
                        temporal: Vec::new(),
                        flow_neighbours: Vec::new(),
                        slate: true,
                        // Nothing was decoded, so nothing was extracted.
                        channels: None,
                        shutter: Vec::new(),
                        shutter_flow: None,
                        cuts: Vec::new(),
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
                // (the layer's own property is the only map). Its
                // interpolation policy, which sits beside that map rather than
                // inside it, decides nearest vs blend.
                let source_time = layer.source_time_at(lt);
                use lumit_core::retime::Interpolation;
                // Flow only engages where it can help:
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
                let flow_neighbours =
                    lumit_core::fx::temporal::stack_flow_wants(&layer.effects, layer.switches.fx);
                // A layer that needs flow decodes at its own width whatever the
                // preview tier says: flow measured on a shrunk decode is
                // a different measurement, not the same one smaller. Must match
                // `Stamper::stamp`'s `native` exactly, or the frame's name lies
                // about the width of the pixels in it.
                let native = flow.is_some() || !flow_neighbours.is_empty();
                let target_width = if native {
                    None
                } else {
                    quality.target_width(nat_w)
                };
                let target_width = fit_texture(target_width, nat_w, nat_h);
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
                let temporal = if lumit_core::fx::temporal::stack_is_temporal(
                    &layer.effects,
                    layer.switches.fx,
                ) {
                    let comp_dt = 1.0 / comp.frame_rate.fps().max(1.0);
                    // Asked at the layer's frame, the unit a plugin counts
                    // in, and the same number the frame key asks at.
                    lumit_core::fx::temporal::stack_temporal_window(
                        &layer.effects,
                        layer.switches.fx,
                        lt / comp_dt,
                        1.0 / comp_dt,
                    )
                    .into_iter()
                    .filter(|&o| o != 0)
                    .map(|o| {
                        let nlt = lt + f64::from(o) * comp_dt;
                        let nst = layer.source_time_at(nlt);
                        let (nf, _) =
                            lumit_core::pixels::frame_pick(nst, fps, src_frames, false, None);
                        (o, nf)
                    })
                    .collect()
                } else {
                    Vec::new()
                };
                // Accumulation motion blur above this layer (docs/08 §3.26):
                // the clip at each moment of the open shutter, so the
                // sub-frame re-renders smear footage motion and not only
                // transforms. Each moment is picked the way the Blend policy
                // picks, so a moment on a real frame is that frame alone and
                // one between two is a crossfade of both. Only a layer whose
                // Retime uses Flow gets its moments synthesised by flow, with
                // its own settings: flow can tear on footage it cannot
                // measure, and it runs only where the user switched it on.
                // Empty for every layer no such adjustment covers.
                let mut shutter: Vec<crate::decode::ShutterSample> = shutter_offsets[idx]
                    .iter()
                    .map(|&off| {
                        let slt = lumit_core::time::layer_time(
                            sample_times[idx] + off * comp_dt,
                            layer.start_offset.0,
                        );
                        let sst = layer.source_time_at(slt);
                        let (source_frame, blend) =
                            lumit_core::pixels::frame_pick(sst, fps, src_frames, true, sample_fps);
                        crate::decode::ShutterSample {
                            offset: off,
                            source_frame,
                            blend,
                        }
                    })
                    .collect();
                // And the clip at each moment a temporal effect above builds
                // it again at. A Posterize-held clip stays held.
                if sample_times[idx] == t {
                    push_moments(
                        &mut shutter,
                        &layer_moments[idx],
                        t,
                        comp_dt,
                        fps,
                        src_frames,
                        |tau| {
                            layer.source_time_at(lumit_core::time::layer_time(
                                tau,
                                layer.start_offset.0,
                            ))
                        },
                    );
                }
                let shutter_flow = match &layer.interpolation {
                    Interpolation::Flow(p) if !shutter.is_empty() => Some(p.clone()),
                    _ => None,
                };
                jobs.push(CompJob {
                    layer: layer.id,
                    at: t,
                    item: *item,
                    source: media_source(doc, *item, media),
                    source_frame,
                    target_width,
                    natural_w: nat_w,
                    natural_h: nat_h,
                    blend,
                    flow,
                    temporal,
                    // Flow motion blur / Datamosh measure motion between
                    // this frame and their requested neighbour (already
                    // in `temporal`) — one entry each when both are live,
                    // since they want opposite directions.
                    flow_neighbours,
                    slate: false,
                    channels: lumit_core::fx::temporal::stack_extracted_channels(
                        &layer.effects,
                        layer.switches.fx,
                    ),
                    shutter,
                    shutter_flow,
                    cuts: Vec::new(),
                });
            }
        }
    }
}

/// The decode jobs a **node graph's** Read boxes need (docs/impl/
/// node-graph-comp.md §2.1): one per Read of footage, keyed by the box's own
/// id - which is what the draw builder looks its pixels up by - and a
/// recursion into a Read of a comp under the guard a Precomp layer takes.
///
/// A Read box is a layer at default placement with no Retime, no effects and
/// no interpolation of its own, so everything the layer walk does beyond the
/// frame pick is inert here: no Posterize hold, no accumulation shutter, no
/// temporal neighbours, no flow. What is left is the proxy resolution, the
/// slate and the frame pick, and those are the walk's own three.
///
/// A picture Input's **preview item** is planned by the same road, since it
/// draws as a Read of that item - unless `pictures_fed` says a host is feeding
/// the pictures, where no preview stands in and nothing of it is decoded
/// (§5.11).
///
/// `moments` are [`collect_comp_jobs`]'s: the other times a temporal effect
/// further up builds this comp again at.
#[allow(clippy::too_many_arguments)]
fn collect_graph_jobs(
    ctx: &PlanContext<'_>,
    comp: &Composition,
    graph: &lumit_core::comp_graph::CompGraph,
    t: f64,
    moments: &[f64],
    jobs: &mut Vec<CompJob>,
    visited: &mut Vec<Uuid>,
    pictures_fed: bool,
) {
    let PlanContext {
        doc,
        shared,
        quality,
        probes,
        held,
    } = *ctx;
    // Only what the Output reaches is fetched, and a Switch whose Index the box
    // alone settles, the same at every moment, is followed down the socket it
    // shows. A wired Index is only known to the builder, so all of its sockets
    // are fetched.
    let picks: Vec<(Uuid, i64)> = graph
        .nodes
        .iter()
        .filter_map(|node| match node {
            lumit_core::comp_graph::GraphNode::Fx(inst)
                if inst.effect.match_name == lumit_core::comp_graph::SWITCH =>
            {
                let at = graph.plain_switch_index(inst, t)?;
                moments
                    .iter()
                    .all(|m| graph.plain_switch_index(inst, *m) == Some(at))
                    .then_some((inst.id, at))
            }
            _ => None,
        })
        .collect();
    let live = graph
        .output_id()
        .map(|output| graph.cone_picked(output, &picks))
        .unwrap_or_default();
    for node in graph.nodes.iter().filter(|n| live.contains(&n.id())) {
        // A nested Node graph box lowers the graph it names into this plan
        // (§2.3), so the inner Read boxes' footage is planned here too.
        if let lumit_core::comp_graph::GraphNode::Fx(inst) = node {
            nested_graph_jobs(ctx, inst, t, moments, jobs, visited);
            continue;
        }
        let (id, item) = match node {
            lumit_core::comp_graph::GraphNode::Read { id, item, .. } => (id, item),
            lumit_core::comp_graph::GraphNode::Input { id, input } => {
                match input.preview.as_ref().filter(|_| !pictures_fed) {
                    Some(item) => (id, item),
                    None => continue,
                }
            }
            _ => continue,
        };
        // An item somebody deleted, or a folder: the box draws transparent and
        // has nothing to decode.
        let Some(project_item) = doc.item(*item) else {
            continue;
        };
        let Some(layer) = lumit_core::comp_graph::read_layer(*id, project_item, comp) else {
            continue;
        };
        match &layer.kind {
            LayerKind::Precomp { comp: nested_id } => {
                if visited.contains(nested_id) {
                    continue; // cycle guard
                }
                let Some(nested) = doc.comp(*nested_id) else {
                    continue;
                };
                // A comp with layers is built again at the same moments.
                //
                // ponytail: a node graph read here is lowered into this
                // graph's plan, which fetches by this comp's boxes alone, so
                // its footage holds. The same goes for a Node graph box and a
                // Node graph effect. Build each through its own comp if a
                // graph inside a graph ever needs to move.
                let moments: &[f64] = if nested.graph.is_some() { &[] } else { moments };
                // A held nested frame wants no decodes: the realiser will
                // serve the texture and never look at the pixels. A rebuild
                // at another moment is made from pixels, as a Precomp's is.
                if moments.is_empty()
                    && held
                        .zip(shared)
                        .is_some_and(|(held, read)| held(read, nested, t, None))
                {
                    continue;
                }
                visited.push(*nested_id);
                collect_comp_jobs(ctx, nested, t, moments, jobs, visited, false);
                visited.pop();
            }
            LayerKind::Footage { item } => {
                // The one proxy resolution point, as the layer walk asks it.
                let Some((media, probe)) = crate::source::effective_media(doc, probes, *item)
                else {
                    continue;
                };
                // Missing media still draws (docs/07 §3.3): a slate at comp
                // size, because a file we cannot open has no size to report.
                if probe.slates() {
                    jobs.push(CompJob {
                        channels: None,
                        layer: *id,
                        at: t,
                        item: *item,
                        source: media_source(doc, *item, media),
                        source_frame: 0,
                        target_width: None,
                        natural_w: comp.width,
                        natural_h: comp.height,
                        blend: None,
                        flow: None,
                        temporal: Vec::new(),
                        flow_neighbours: Vec::new(),
                        slate: true,
                        shutter: Vec::new(),
                        shutter_flow: None,
                        cuts: Vec::new(),
                    });
                    continue;
                }
                // Not probed yet, or audio-only: no picture. Retried once the
                // probe lands.
                let Some((fps, nat_w, nat_h, src_frames)) = probe.video() else {
                    continue;
                };
                let (source_frame, blend) =
                    lumit_core::pixels::frame_pick(t, fps, src_frames, false, None);
                // A Read plays its file at comp time, at every moment too.
                let mut shutter = Vec::new();
                let comp_dt = 1.0 / comp.frame_rate.fps().max(1.0);
                push_moments(&mut shutter, moments, t, comp_dt, fps, src_frames, |tau| {
                    tau
                });
                jobs.push(CompJob {
                    channels: None,
                    layer: *id,
                    at: t,
                    item: *item,
                    source: media_source(doc, *item, media),
                    source_frame,
                    target_width: quality.target_width(nat_w),
                    natural_w: nat_w,
                    natural_h: nat_h,
                    blend,
                    flow: None,
                    temporal: Vec::new(),
                    flow_neighbours: Vec::new(),
                    slate: false,
                    shutter,
                    shutter_flow: None,
                    cuts: Vec::new(),
                });
            }
            // A solid rasterises where it is drawn, and no other kind of item
            // can be read into a graph.
            _ => {}
        }
    }
}

/// The jobs one **Node graph effect** needs: the graph it names, planned under
/// the guard a Precomp layer takes. The three places one can sit all come
/// through here - a layer's stack, a live group's header, and a box nested
/// inside another graph - so none of them can drift apart. The builder lowers
/// that graph's Read boxes into this plan, and a Read whose pixels nobody
/// decoded draws transparent.
///
/// A box that is bypassed, unbound, cyclic or naming a comp that is gone has
/// nothing to plan, which is the passthrough the walk renders.
///
/// `moments` are the other times the graph is lowered at, on the clock `t` is
/// on, so its Reads of footage are fetched at those too.
fn nested_graph_jobs(
    ctx: &PlanContext<'_>,
    inst: &lumit_core::model::EffectInstance,
    t: f64,
    moments: &[f64],
    jobs: &mut Vec<CompJob>,
    visited: &mut Vec<Uuid>,
) {
    if !inst.enabled || inst.effect.match_name != lumit_core::comp_graph::NODE_GRAPH {
        return;
    }
    let Some(named) = lumit_core::fx::effects::node_graph::comp_of(inst) else {
        return;
    };
    if visited.contains(&named) {
        return;
    }
    let Some(nested) = ctx.doc.comp(named) else {
        return;
    };
    visited.push(named);
    match nested.graph.as_ref() {
        // Applied or nested, so every picture Input arrives on a socket and no
        // preview item is drawn or decoded (§5.11).
        Some(graph) => collect_graph_jobs(ctx, nested, graph, t, moments, jobs, visited, true),
        // A comp with layers: the dangling reference the walk renders as a
        // passthrough, planned as the Precomp it looks like.
        None => collect_comp_jobs(ctx, nested, t, moments, jobs, visited, false),
    }
    visited.pop();
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
    planned(doc, None, comp, t, quality, probes, None)
}

/// [`plan_comp_frame`] for the document as the renderer shares it, with the
/// nested-frame answerer ([`PlanContext::held`]). What a frame is planned by,
/// so a copy of a graphics template is planned under its own values.
#[must_use]
pub fn plan_comp_frame_held(
    doc: &std::sync::Arc<Document>,
    comp: &Composition,
    t: f64,
    quality: Quality,
    probes: &dyn SourceProbes,
    held: Option<HeldNested<'_>>,
) -> Vec<CompJob> {
    planned(doc, Some(doc), comp, t, quality, probes, held)
}

fn planned(
    doc: &Document,
    shared: Option<&std::sync::Arc<Document>>,
    comp: &Composition,
    t: f64,
    quality: Quality,
    probes: &dyn SourceProbes,
    held: Option<HeldNested<'_>>,
) -> Vec<CompJob> {
    let ctx = PlanContext {
        doc,
        shared,
        quality,
        probes,
        held,
    };
    let mut jobs = Vec::new();
    let mut visited = vec![comp.id];
    collect_comp_jobs(&ctx, comp, t, &[], &mut jobs, &mut visited, false);
    crate::placed::merged(jobs)
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
                && x.at.to_bits() == y.at.to_bits()
                && x.item == y.item
                && x.source_frame == y.source_frame
                && x.target_width == y.target_width
                && x.slate == y.slate
                && x.blend == y.blend
                && x.flow == y.flow
                && x.temporal == y.temporal
                && x.flow_neighbours == y.flow_neighbours
                && x.channels == y.channels
                && x.shutter == y.shutter
                && x.shutter_flow == y.shutter_flow
                && x.cuts
                    .iter()
                    .map(Cut::name)
                    .eq(y.cuts.iter().map(Cut::name))
        })
}

impl Cut {
    /// What this asks for, as numbers that compare and hash: which kind, the
    /// offset or the gain, and the other clip's own content name.
    pub(crate) fn name(&self) -> (u8, u64, Option<u128>) {
        match self {
            Cut::Neighbour(o, job) => (0, f64::from(*o).to_bits(), Some(job.source_key())),
            Cut::Moment(o, job) => (1, o.to_bits(), job.as_ref().map(CompJob::source_key)),
            Cut::Under(gain, job) => (2, gain.to_bits(), job.as_ref().map(CompJob::source_key)),
        }
    }
}

impl CompJob {
    /// The content name of the pixels this job decodes: a hash of
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
        h.update(self.source.path.to_string_lossy().as_bytes());
        // A run of stills read at a different rate is a different picture at
        // the same frame number, so the rate is part of the name.
        let (num, den) = self.source.sequence_fps.unwrap_or((0, 0));
        h.update(&num.to_le_bytes());
        h.update(&den.to_le_bytes());
        // And so is a file read another way. Left out while it is read as it
        // is, so every name from before interpretation existed still stands.
        if !self.source.interpret.is_plain() {
            h.update(format!("{:?}", self.source.interpret).as_bytes());
        }
        if let Some(pass) = &self.source.render_pass {
            h.update(b"pass/");
            h.update(pass.as_bytes());
        }
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
            synthesis_key::feed_synthesis(&mut h, flow);
        }
        for (offset, frame) in &self.temporal {
            h.update(&offset.to_le_bytes());
            h.update(&frame.to_le_bytes());
        }
        // `i32::MIN` is not a reachable offset, so "no flow consumer" cannot
        // collide with a real one — and a stack with a single consumer keeps
        // exactly the name it had before this became a list.
        if self.flow_neighbours.is_empty() {
            h.update(&i32::MIN.to_le_bytes());
        }
        for (offset, want) in &self.flow_neighbours {
            h.update(&offset.to_le_bytes());
            h.update(want.name_tag());
        }
        // The extracted channels name the pixels as surely as the frame number
        // does: the same file at the same frame is a different picture read as
        // `Z` than read as RGB, and without this the cache would hand back
        // whichever was asked for first.
        if let Some(slots) = &self.channels {
            h.update(b"channels/");
            for slot in slots {
                h.update(slot.as_deref().unwrap_or("").as_bytes());
                h.update(&[0]);
            }
        }
        // The shutter moments are content too: the same frame decoded for a
        // wider shutter carries different in-between pictures.
        for s in &self.shutter {
            h.update(b"shutter/");
            h.update(&s.offset.to_le_bytes());
            h.update(&s.source_frame.to_le_bytes());
            if let Some((ceil, weight)) = s.blend {
                h.update(&ceil.to_le_bytes());
                h.update(&weight.to_le_bytes());
            }
        }
        if let Some(flow) = &self.shutter_flow {
            h.update(b"shutter-flow/");
            h.update(&bincode::serialize(flow).unwrap_or_default());
            synthesis_key::feed_synthesis(&mut h, flow);
        }
        // Another clip's frame is content too, and so is a gap.
        for (kind, offset, clip) in self.cuts.iter().map(Cut::name) {
            h.update(b"cut/");
            h.update(&[kind, u8::from(clip.is_some())]);
            h.update(&offset.to_le_bytes());
            h.update(&clip.unwrap_or(0).to_le_bytes());
        }
        let mut k = [0u8; 16];
        k.copy_from_slice(&h.finalize().as_bytes()[..16]);
        u128::from_le_bytes(k)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;

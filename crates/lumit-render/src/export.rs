//! Export (docs/06-RENDER-PIPELINE.md §7): render every work-area frame
//! through the compositor at full resolution and encode to H.264/mp4.
//!
//! In plain terms: the same pixels the Viewer shows, written to a file — the
//! preview-equals-export promise holds because this path reuses the
//! identical colour engine and compositor. Precomp layers render recursively:
//! the nested comp becomes a texture the parent composites like any other
//! source. Runs on its own thread with its own decoders; progress
//! streams back; cancel is checked every frame.

use lumit_audio::mix::MixPlan;
use lumit_audio::stream::Source;
use lumit_core::model::{Document, LayerKind, ProjectItem};
pub use lumit_core::pixels::{px_tile, solid_rgba, srgb_decode, srgb_encode};
use lumit_core::retime::Interpolation;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use uuid::Uuid;

pub enum ExportEvent {
    /// Which encoder the ladder settled on ("NVENC", "software x264", …),
    /// sent once the file is open.
    Encoder(&'static str),
    Progress {
        frame: usize,
        total: usize,
    },
    Done(PathBuf),
    Failed(String),
}

pub struct ExportHandle {
    pub events: Receiver<ExportEvent>,
    cancel: Arc<AtomicBool>,
}

impl ExportHandle {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// Everything the export thread needs about one footage item.
#[derive(Clone)]
pub struct ItemInfo {
    /// Where the pixels come from: one file, or the numbered run of stills the
    /// item names one file of.
    pub source: lumit_media::MediaSource,
    pub fps: f64,
    pub frames: usize,
    /// The file could not be found (docs/07 §3.3): render the test-bar slate
    /// at this size rather than decoding. `Some((w, h))` carries the comp's
    /// dimensions — the preview sizes a missing layer the same way, since a
    /// file we cannot open has no size of its own, and the two must agree or
    /// the layer's geometry would differ between them. Export must match the
    /// preview: an export that quietly dropped a missing layer to
    /// black while the Viewer showed bars would hide the mistake in the
    /// delivered file, which is precisely what the slate prevents.
    pub missing: Option<(u32, u32)>,
}

/// One audio-bearing layer, as the export thread needs it: where its file
/// is, its comp-timeline span, its start offset, and its Volume (the same
/// set the preview mix uses, so export audio matches playback).
#[derive(Clone, PartialEq)]
pub struct AudioJob {
    /// The footage item the audio comes from — the key the preview path uses
    /// to reuse an already-decoded buffer instead of re-decoding the file.
    pub item: uuid::Uuid,
    /// **The mixer strip this sound belongs to**: the layer *of the comp
    /// being mixed* that carries it (docs/09 §3.1).
    ///
    /// For a footage layer that is its own id. For sound arriving through a
    /// Precomp layer it is the **Precomp layer's** id, not the inner
    /// footage layer's — the mixer draws the rows of the comp in front of
    /// you, and a nested comp is one row on it however many sources are
    /// inside. Several jobs therefore share a strip, which is exactly what
    /// summing them onto one meter means.
    pub layer: uuid::Uuid,
    /// **Which clip of that layer** the sound came through, where it came
    /// through one: the clip `sequence_jobs` made this job from, and `None`
    /// for every whole-layer job. Only a filtered reading looks at it
    /// (docs/impl/audio-nodes.md §2); the mixer sums the jobs it is handed
    /// whatever clips they came from.
    pub clip: Option<uuid::Uuid>,
    pub path: PathBuf,
    pub in_s: f64,
    pub out_s: f64,
    pub offset_s: f64,
    /// The layer's Volume property (dB, docs/09 §6): static values become a
    /// constant gain; keyframed ones bake to a control-rate envelope.
    pub volume: lumit_core::anim::Property,
    /// The layer's Pan property (−100..+100, docs/09 §6): a
    /// constant-power stereo balance, folded into the same gain stage the
    /// Volume rides.
    pub pan: lumit_core::anim::Property,
    /// Enclosing Precomp layers, outermost first — a precomp's Volume and
    /// Pan act on everything inside it, so both multiply through the chain.
    pub carriers: Vec<Carrier>,
    /// The head and tail ramps of a **Sequence clip**. `None` for a
    /// whole-layer job, which has no join to fade across.
    pub fade: Option<ClipFade>,
    /// The layer's driver chain, where one is wired onto the Layer out's
    /// Volume socket (the *Duck under* wire). `None` for the ordinary layer,
    /// whose Volume keyframes answer as ever.
    pub driven: Option<Arc<DrivenVolume>>,
    /// The layer's **audio insert chain** — the audio-typed effects in its
    /// stack, ahead of Volume and Pan. `None` for a layer carrying no
    /// effects at all, which is what keeps every mix without a plugin
    /// byte-identical to the one before this field existed.
    pub chain: Option<Arc<AudioChain>>,
    /// The **clip's own** insert chain, where the clip carries a stack of its
    /// own and has not bypassed it (docs/impl/audio-timeline.md §4). It runs
    /// before [`Self::chain`]: the clip's rack, then the row's, which is the
    /// order the two racks read on screen. `None` for every whole-layer job.
    pub clip_chain: Option<Arc<AudioChain>>,
}

/// A wire onto a layer's Volume (the Audio workspace board's *Duck under*):
/// everything [`volume_bake`] needs to ask the driver chain for the decibels
/// at each control-rate step, carried on the job because the bake runs long
/// after the document walk that found the wire.
///
/// The chain is evaluated with the same tap a visual driver reads
/// ([`crate::audio_tap::DocumentAudio`]), built **pre-duck**: a chain that
/// reads "this comp" hears the mix at everyone's keyframed Volume, never
/// through another duck — one level of ducking is heard, and a duck driven by
/// a duck would otherwise be this function calling itself forever.
pub struct DrivenVolume {
    pub doc: Arc<lumit_core::model::Document>,
    /// The comp the layer sits in — not necessarily the comp being mixed,
    /// because a nested comp's layers duck inside their own timeline.
    pub comp: Uuid,
    pub layer: Uuid,
    pub graph: lumit_core::graph::LayerGraph,
    /// Comp time where the layer's own time 0 sits (its start offset plus the
    /// branch base), so `t − offset_s` is the layer time the chain reads at.
    pub offset_s: f64,
    /// Where the layer's comp starts on the mixed timeline (the branch base),
    /// so `t − base_s` is that comp's own clock for the tap and the context.
    pub base_s: f64,
}

impl DrivenVolume {
    /// The decibels the chain answers at mixed-timeline time `t`, or `None`
    /// when the wire is broken or bypassed (the keyframes come back).
    #[must_use]
    pub fn db_at(&self, t: f64) -> Option<f64> {
        let comp = self.doc.comp(self.comp)?;
        let t_comp = t - self.base_s;
        let context = Arc::new(lumit_core::expression::ExpressionContext {
            document: Arc::clone(&self.doc),
            comp: Some(self.comp),
            layer: Some(self.layer),
            comp_time: t_comp,
            current_depth: 0,
            inputs: None,
        });
        let tap = crate::audio_tap::DocumentAudio::pre_duck(&self.doc, comp, t_comp);
        lumit_core::fx::driven_volume_db(&self.graph, t - self.offset_s, context, Some(&tap))
    }
}

/// Two jobs compare equal when they would bake the same sound. For the driven
/// chain that is the wiring and its drivers' values — the document handle is
/// deliberately not compared, because two snapshots holding the same graph
/// bake the same envelope.
impl PartialEq for DrivenVolume {
    fn eq(&self, other: &Self) -> bool {
        self.comp == other.comp
            && self.layer == other.layer
            && self.graph == other.graph
            && self.offset_s == other.offset_s
            && self.base_s == other.base_s
    }
}

/// A layer's **audio insert chain**, as the mixer needs it
/// (docs/impl/audio-plugins.md §2).
///
/// # In plain terms
///
/// "The stack is the rack": an audio plugin is an entry in the layer's ordinary
/// effect stack, and this is that stack carried down to where the sound is,
/// with the two things the bake cannot look up for itself — the document the
/// plugin's rows are evaluated against, and where the layer's own clock sits on
/// the mixed timeline.
///
/// Which entries are audio is not decided here: the catalogue is asked, once,
/// when the chain opens ([`lumit_core::fx::EffectDef::open_audio`]). A stack
/// full of blurs opens nothing and the sound goes through untouched.
///
/// A **Precomp** layer's chain is a chain like any other, and it rides on the
/// carrier rather than on a job ([`Carrier::chain`]): everything arriving
/// through that layer is summed first and the rack hears the sum, because gain
/// distributes over a sum and a compressor does not. A **Sequence** layer's
/// chain still runs per clip rather than on the row's mixed output, so a reverb
/// does not tail across a join; that one wants the same sum, a row at a time.
pub struct AudioChain {
    pub doc: Arc<lumit_core::model::Document>,
    /// The comp the layer sits in — not necessarily the comp being mixed.
    pub comp: Uuid,
    pub layer: Uuid,
    /// The layer's whole effect stack, in order.
    pub effects: Vec<lumit_core::model::EffectInstance>,
    /// The layer's driver graph, so a **wired** plugin parameter reads the wire
    /// instead of its keyframes, exactly as a wired effect parameter in the
    /// picture does.
    pub graph: lumit_core::graph::LayerGraph,
    /// Comp time where the layer's own time 0 sits, so `t − offset_s` is the
    /// layer time its rows are read at.
    pub offset_s: f64,
    /// Where the layer's comp starts on the mixed timeline.
    pub base_s: f64,
}

/// As [`DrivenVolume`]'s: two chains that would process the same sound compare
/// equal, and the document handle is not part of that.
impl PartialEq for AudioChain {
    fn eq(&self, other: &Self) -> bool {
        self.comp == other.comp
            && self.layer == other.layer
            && self.effects == other.effects
            && self.graph == other.graph
            && self.offset_s == other.offset_s
            && self.base_s == other.base_s
    }
}

/// Open the chain and play `samples` (interleaved stereo, the job's placed
/// span) through it — the **one** function the live plan and the export both
/// call, which is what makes preview == export a fact about the code rather
/// than an argument about it.
///
/// `start_frame` is where the span lands on the mixed timeline, and is what the
/// per-block parameter values are read at. `offline` says this is an export: no
/// deadline, and the plugin may take its slower, better path.
///
/// `None` when the layer's stack holds nothing the catalogue will open as
/// audio, which is the ordinary answer and the one that leaves the decoded
/// buffer untouched. Otherwise the processed span and the chain's summed
/// latency, in frames — the caller places the sound that many frames **earlier**
/// so the wet lands where the dry did.
///
/// The span that comes back may be **longer** than the one that went in: a
/// chain with a tail (a reverb's decay, an echo's repeats) runs on past its
/// input, and the caller places the whole of what it is given, so the decay
/// simply sums with whatever follows.
///
/// ponytail: the whole span is processed here, at plan-build time, rather than
/// a block at a time into a lookahead ring on a chain worker
/// (docs/impl/audio-plugins.md §3). The ring exists to keep the realtime
/// callback off another process; a span already rendered keeps it off even more
/// firmly. The plan streams its files now ([`lumit_audio::stream`]) and a
/// racked clip is the one thing in it that is still held whole: its processed
/// span stays in memory for as long as the plan does, outside the block
/// budget, and [`RackBakes`] keeps it from being made again on every edit.
/// The ring is the upgrade, with this function as the block loop it wraps,
/// when a cut carries enough racked sound for that memory to matter.
#[must_use]
pub fn chain_bake(
    chain: &AudioChain,
    samples: &[f32],
    start_frame: i64,
    rate: u32,
    offline: bool,
) -> Option<(Vec<f32>, u32)> {
    use lumit_core::fx::AUDIO_BLOCK_FRAMES;

    let frames = samples.len() / 2;
    if frames == 0 {
        return None;
    }
    // Open first, bake second. Opening answers the question "are you audio?",
    // and only once every link has answered is the chain's summed latency —
    // and therefore how many blocks the run is — known. Baking a whole
    // envelope for a blur that turns out not to be a plugin would be work for
    // nothing, so opening is handed one time point rather than the envelope.
    let mut opened = Vec::new();
    for instance in chain.effects.iter().filter(|e| e.enabled) {
        let Some(def) = lumit_core::fx::BUILTIN_DEFS.get(&instance.effect.match_name) else {
            continue;
        };
        let first = bake_values(chain, instance, def, start_frame, 1, rate)
            .pop()
            .unwrap_or_default();
        if let Some(processor) =
            def.open_audio(instance.plugin_state_bytes(), &first, rate, offline)
        {
            opened.push((instance, def, processor));
        }
    }
    if opened.is_empty() {
        return None;
    }
    let latency = opened
        .iter()
        .map(|(_, _, processor)| processor.latency() as usize)
        .sum::<usize>();
    // The tail belongs in the block count too: `run_chain` runs the extra
    // frames, and a row automated past the input's end still wants a value for
    // every one of them.
    let tail = opened
        .iter()
        .map(|(_, _, processor)| processor.tail() as usize)
        .sum::<usize>();
    let blocks = (frames + latency + tail).div_ceil(AUDIO_BLOCK_FRAMES);
    let ids: Vec<Uuid> = opened.iter().map(|(instance, _, _)| instance.id).collect();
    let links: Vec<lumit_core::fx::ChainLink> = opened
        .into_iter()
        .map(|(instance, def, processor)| lumit_core::fx::ChainLink {
            values: bake_values(chain, instance, def, start_frame, blocks, rate),
            processor,
        })
        .collect();
    let out = lumit_core::fx::run_chain(&links, samples);
    // The badge, per link (AP5, docs/12 §2.3): a link that shipped any block
    // dry files the host's own sentence against *its* instance — the same
    // table an OFX frame files into, read by the same `badge_of` — and a link
    // whose every block came back takes any stale badge off. Filed here, on
    // the bake, because this is the one place that knows which link refused.
    for ((id, dry), link) in ids.iter().zip(&out.dry_by_link).zip(&links) {
        let sentence = (*dry > 0).then(|| {
            link.processor
                .last_error()
                .unwrap_or_else(|| "the plugin did not process this sound".to_owned())
        });
        crate::gpufx::ofx::note(*id, sentence);
    }
    Some((out.samples, out.latency))
}

/// Run a job's **two** chains over its placed span: the clip's own rack
/// first, then the row's on what came out of it
/// (docs/impl/audio-timeline.md §4).
///
/// The one function both mixers call, for the same reason [`chain_bake`] is:
/// the order of the racks and the sum of their latencies are facts about the
/// code rather than something the preview and the export each decide. `None`
/// when neither chain opens anything as audio, which leaves the decoded buffer
/// exactly as it was.
///
/// The row's chain is read at `start_frame` less the clip chain's latency,
/// because that is where the sound it is handed now begins; the returned
/// latency is the two summed, and the caller places the run that many frames
/// earlier.
#[must_use]
pub fn job_bake(
    job: &AudioJob,
    samples: &[f32],
    start_frame: i64,
    rate: u32,
    offline: bool,
) -> Option<(Vec<f32>, u32)> {
    let clip = job
        .clip_chain
        .as_ref()
        .and_then(|chain| chain_bake(chain, samples, start_frame, rate, offline));
    let (dry, head): (&[f32], u32) = match &clip {
        Some((wet, latency)) => (wet, *latency),
        None => (samples, 0),
    };
    let layer = job
        .chain
        .as_ref()
        .and_then(|chain| chain_bake(chain, dry, start_frame - i64::from(head), rate, offline));
    match layer {
        Some((wet, latency)) => Some((wet, latency.saturating_add(head))),
        None => clip,
    }
}

/// What one effect instance's rows hold at each block start.
///
/// A row reads its own keyframes unless a wire feeds it, in which case it reads
/// the wire — the same substitution the picture's resolve makes, held to the
/// same declared hard range, so a plugin parameter is driven
/// like everything else. An instance with nothing animated and nothing wired
/// bakes **one** entry, which the chain then holds past the end: an
/// un-automated plugin costs no per-block work at all.
///
/// ponytail: the driver walk is given no [`lumit_core::fx::AudioTap`], so an
/// *Audio level* driver wired into a plugin parameter reads nothing here.
/// Handing it the real tap would mix a window of the comp per block — tens of
/// thousands of mixes for a song. Bake the tap's levels once at control rate
/// and index them, when a duck-driven plugin is a thing somebody asks for.
fn bake_values(
    chain: &AudioChain,
    instance: &lumit_core::model::EffectInstance,
    def: &'static dyn lumit_core::fx::EffectDef,
    start_frame: i64,
    blocks: usize,
    rate: u32,
) -> Vec<Vec<(lumit_core::fx::ParamId, f64)>> {
    use lumit_core::fx::{hard_range, ParamId, AUDIO_BLOCK_FRAMES};
    use lumit_core::graph::{InputRef, NodeRef};
    use lumit_core::model::EffectValue;

    /// One automatable row: what it is called, what it holds, and how far it
    /// may be pushed.
    struct Row<'a> {
        id: ParamId,
        property: std::borrow::Cow<'a, lumit_core::anim::Property>,
        hard: (Option<f64>, Option<f64>),
    }

    let rows: Vec<Row<'_>> = def
        .schema()
        .params
        .iter()
        .filter_map(|row| {
            let held = instance.params.iter().find(|p| p.id == row.id)?;
            let property = match &held.value {
                EffectValue::Float(property) => std::borrow::Cow::Borrowed(property),
                // A switch goes to a plugin as nought or one.
                EffectValue::Bool(on) => std::borrow::Cow::Owned(
                    lumit_core::anim::Property::fixed(f64::from(u8::from(*on))),
                ),
                _ => return None,
            };
            Some(Row {
                id: ParamId::new(row.id),
                property,
                hard: hard_range(&row.kind),
            })
        })
        .collect();
    if rows.is_empty() {
        return Vec::new();
    }
    let node = NodeRef::Effect(instance.id);
    let wired =
        chain.graph.edges.iter().any(
            |edge| matches!(&edge.to, InputRef::Param { node: target, .. } if *target == node),
        );
    let animated = wired || rows.iter().any(|row| row.property.is_animated());
    let n = if animated { blocks.max(1) } else { 1 };

    (0..n)
        .map(|block| {
            let t = (start_frame + (block * AUDIO_BLOCK_FRAMES) as i64) as f64 / f64::from(rate);
            let lt = t - chain.offset_s;
            let drivers = if wired {
                let context = Arc::new(lumit_core::expression::ExpressionContext {
                    document: Arc::clone(&chain.doc),
                    comp: Some(chain.comp),
                    layer: Some(chain.layer),
                    comp_time: t - chain.base_s,
                    current_depth: 0,
                    inputs: None,
                });
                lumit_core::fx::resolve_drivers(&chain.graph, lt, context, None)
            } else {
                lumit_core::fx::ResolvedDrivers::default()
            };
            rows.iter()
                .map(|row| {
                    let value = drivers
                        .param(node, row.id)
                        .map(|v| f64::from(v.as_f32()))
                        .unwrap_or_else(|| row.property.value_at(lt));
                    // `max`/`min` rather than `clamp`, which panics on a
                    // reversed pair (14-ENGINEERING-RULES §4).
                    let value = value
                        .max(row.hard.0.unwrap_or(f64::NEG_INFINITY))
                        .min(row.hard.1.unwrap_or(f64::INFINITY));
                    (row.id, value)
                })
                .collect()
        })
        .collect()
}

/// A Sequence clip's own fade ramps, in **comp time**.
///
/// Absolute times rather than durations measured from the job's audible span,
/// because a clip's span can be trimmed by the layer's in and out points
/// while its ramps stay where the join put them. Anchored to the clip, the
/// arithmetic survives every trim without re-basing.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct ClipFade {
    /// Comp time the clip starts, and how long it takes to come up.
    pub start_s: f64,
    pub head_s: f64,
    pub head_shape: lumit_core::sequence::FadeShape,
    /// Comp time the clip ends, and how long it takes to go away.
    pub end_s: f64,
    pub tail_s: f64,
    pub tail_shape: lumit_core::sequence::FadeShape,
    /// The clip's own level in dB (docs/impl/audio-timeline.md §2), carried
    /// here because it is applied exactly where the ramps are.
    pub gain_db: f64,
}

impl ClipFade {
    /// The ramp's gain at comp time `t` — 1.0 anywhere the clip is at full
    /// level.
    ///
    /// Each end reads its own stored shape ([`lumit_core::sequence::FadeShape`],
    /// docs/impl/audio-timeline.md §3), the tail reading the curve backwards
    /// because that is what a fade out is. The default shape is **equal
    /// power** (a quarter-sine, not a straight line), which is what makes a
    /// crossfade hold its level across the join: two opposed sine ramps have
    /// squares that sum to one, so uncorrelated material - two different
    /// shots, the usual case - neither dips nor swells in the middle. A
    /// straight line would dip by 3 dB there, which is the classic hole in the
    /// middle of a dissolve.
    ///
    /// The head and the tail still multiply, so a clip shorter than its two
    /// fades is heard as their product.
    ///
    /// The clip's own gain comes in **after** the ramps, so the ramps rise to
    /// the gain rather than to unity: that is what the line drawn across the
    /// box means, and it is why the gain is here and not another link in the
    /// chain. It carries the fader's own knee, so a clip pulled to the foot of
    /// the box is exactly silent.
    #[must_use]
    pub fn gain_at(&self, t: f64) -> f32 {
        let mut g = 1.0f64;
        if self.head_s > 0.0 {
            g *= self.head_shape.gain((t - self.start_s) / self.head_s);
        }
        if self.tail_s > 0.0 {
            g *= self.tail_shape.gain((self.end_s - t) / self.tail_s);
        }
        g *= f64::from(lumit_audio::mix::db_to_gain(self.gain_db));
        g as f32
    }

    /// Whether either ramp actually does anything.
    ///
    /// The gain is deliberately not in here: it is a constant, and
    /// [`volume_bake`] reads a constant off `gain_at(0.0)` without needing an
    /// envelope for it.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.head_s > 0.0 || self.tail_s > 0.0
    }
}

/// One enclosing Precomp layer's contribution to a job's gain: its Volume,
/// its Pan, and the outer-comp time where its layer time 0 sits (each
/// property is sampled in its own layer time). A Sequence clip whose source
/// is a composition is a carrier too, with the row's Volume and Pan.
#[derive(Clone, PartialEq)]
pub struct Carrier {
    pub volume: lumit_core::anim::Property,
    pub pan: lumit_core::anim::Property,
    pub offset_s: f64,
    /// The ramps and the level of the Sequence clip a nested comp plays
    /// through, on everything the comp sounds. `None` for a Precomp layer.
    pub fade: Option<ClipFade>,
    /// The Precomp layer's own **rack**, where it holds anything that sounds:
    /// the bus chain (docs/09 §3.1). Everything arriving through this
    /// carrier is summed first and run through it, and this carrier's Volume
    /// and Pan then ride on the result, which is the insert-then-fader order
    /// a layer's own rack already reads. `None` on every carrier that is a
    /// gain and nothing more, which is nearly all of them.
    pub chain: Option<std::sync::Arc<AudioChain>>,
}

/// Bake one job's **Volume and Pan** — its own properties times every
/// carrier's — for its placed span: `([left, right] constant gain,
/// envelope)`. All-static chains are exactly their constant product
/// (envelope None); any animated link bakes the whole chain to a ~10 ms
/// control-rate curve, each property sampled in its own layer time
/// (`lt = comp time − its offset`).
///
/// **One stage for both**. A balance is a pair of per-channel gains,
/// so it multiplies into the Volume's gain rather than needing a stage of its
/// own — which is also what makes a Precomp layer's balance compose with the
/// balances inside it, channel by channel.
pub fn volume_bake(
    job: &AudioJob,
    start_frame: i64,
    len: usize,
    rate: u32,
) -> ([f32; 2], Option<lumit_audio::mix::GainEnvelope>) {
    gain_bake(Some(job), &job.carriers, start_frame, len, rate)
}

/// The gain law itself: `own`'s Volume, Pan and fade where a job's own sound
/// is being placed, times `carriers`.
///
/// The bus stage splits one job's chain of gains in two, the carriers inside
/// the bus riding into the sum and the ones outside riding on what the rack
/// gives back, so both halves are asked for here rather than each doing its
/// own arithmetic. `None` for `own` is a summed bus, whose own Volume and Pan were
/// spent on the sources inside it.
fn gain_bake(
    own: Option<&AudioJob>,
    carriers: &[Carrier],
    start_frame: i64,
    len: usize,
    rate: u32,
) -> ([f32; 2], Option<lumit_audio::mix::GainEnvelope>) {
    let gain_at = |t: f64| {
        let mut g = 1.0;
        let mut lr = [1.0f32, 1.0];
        if let Some(job) = own {
            // A wired Volume overrides its keyframes, exactly as a wired effect
            // parameter does; a broken or bypassed chain answers `None`
            // and the keyframes come back.
            let volume_db = job
                .driven
                .as_ref()
                .and_then(|d| d.db_at(t))
                .unwrap_or_else(|| job.volume.value_at(t - job.offset_s));
            g = lumit_audio::mix::db_to_gain(volume_db);
            lr = lumit_audio::mix::pan_gains(job.pan.value_at(t - job.offset_s));
            // A clip's own crossfade ramps multiply in too: they are the
            // join's, not the layer's, and they ride on whatever the Volume is
            // doing.
            if let Some(fade) = &job.fade {
                g *= fade.gain_at(t);
            }
        }
        for c in carriers {
            g *= lumit_audio::mix::db_to_gain(c.volume.value_at(t - c.offset_s));
            if let Some(fade) = &c.fade {
                g *= fade.gain_at(t);
            }
            let cp = lumit_audio::mix::pan_gains(c.pan.value_at(t - c.offset_s));
            lr[0] *= cp[0];
            lr[1] *= cp[1];
        }
        [lr[0] * g, lr[1] * g]
    };
    let animated = own.is_some_and(|job| {
        job.volume.is_animated()
            || job.pan.is_animated()
            || job.fade.is_some_and(|f| f.is_active())
            // A driven Volume follows the sound, which moves whether or not any
            // keyframe does — always an envelope, never a constant.
            || job.driven.is_some()
    }) || carriers.iter().any(|c| {
        c.volume.is_animated() || c.pan.is_animated() || c.fade.is_some_and(|f| f.is_active())
    });
    if !animated {
        return (gain_at(0.0), None);
    }
    let stride = (rate / 100).max(1);
    let n = len / stride as usize + 2;
    let points = (0..n)
        .map(|p| {
            let t = (start_frame + p as i64 * i64::from(stride)) as f64 / f64::from(rate);
            gain_at(t)
        })
        .collect();
    (
        [1.0, 1.0],
        Some(lumit_audio::mix::GainEnvelope { stride, points }),
    )
}

/// One job on its way into the mix, as the bus stage takes it: the run about
/// to be placed (its clip and layer racks already run) and where it lands.
pub struct PlacedJob<'a> {
    pub job: &'a AudioJob,
    pub start_frame: i64,
    pub samples: &'a [f32],
}

/// Whether a job's sound arrives through a Precomp layer carrying a rack, and
/// so has to be in hand to be summed. Every other job is placed by its
/// numbers alone.
#[must_use]
pub fn in_a_bus(job: &AudioJob) -> bool {
    bus_at(job, 0).is_some()
}

/// Whose sound one [`MixRun`] is.
pub enum RunOf {
    /// One job's own, at this index of what the stage was handed: the caller
    /// places the samples it already has.
    Job(usize),
    /// A **bus**: everything arriving through one Precomp layer, summed and
    /// run through that layer's rack.
    Bus(Vec<f32>),
}

/// One run for the mixer to place, with the gain still to ride on it.
pub struct MixRun {
    /// Which of the jobs handed to [`bus_runs`] this run stands where: the
    /// job itself, or the first job of the bus. Runs are summed in the order
    /// of the jobs, and a mixer that stages only its buses puts each one back
    /// in line by this.
    pub first: usize,
    /// The mixer strip this run meters onto. A bus meters on the Precomp
    /// layer's own strip, which is the row the board draws it as.
    pub layer: Uuid,
    pub start_frame: i64,
    pub of: RunOf,
    pub gain: [f32; 2],
    pub envelope: Option<lumit_audio::mix::GainEnvelope>,
}

/// **The bus stage** (docs/09 §3.1): fold every job arriving through a Precomp
/// layer that carries a rack into one sum, run the rack over it, and hand back
/// the runs the mixer places. It is the one function the live plan and the
/// export both call, so a rack on a nested comp sounds the same in both.
///
/// A rack cannot be pushed down onto the sources the way Volume and Pan are:
/// gain distributes over a sum and a compressor does not, so the sum has to
/// exist before the rack can hear it. Inside the bus a source carries its own
/// gains and the carriers below the rack, the nested comp's **master fader**
/// among them, since that is a stage on that comp's own sum and therefore
/// ahead of anything the parent inserts. Outside it, the Precomp layer's
/// Volume and Pan and every enclosing carrier ride on what the rack gives
/// back, which is the insert-then-fader order a layer's own rack reads.
///
/// Nested buses are the same act one level down: a bus is summed from the runs
/// inside it, and one of those may itself be a bus.
///
/// A job under no rack at all comes back as [`RunOf::Job`] with exactly the
/// gain [`volume_bake`] bakes, so a mix with no rack in it is the mix it was.
#[must_use]
pub fn bus_runs(placed: &[PlacedJob<'_>], rate: u32, offline: bool) -> Vec<MixRun> {
    let all: Vec<usize> = (0..placed.len()).collect();
    runs_at(placed, &all, 0, rate, offline)
}

/// The first carrier at or past `depth` that holds a rack: where this job's
/// sound is summed on its way out.
fn bus_at(job: &AudioJob, depth: usize) -> Option<usize> {
    job.carriers
        .iter()
        .enumerate()
        .skip(depth)
        .find_map(|(at, c)| c.chain.is_some().then_some(at))
}

/// The runs inside one bus: `which` indexes [`bus_runs`]'s input and `depth`
/// is the first carrier whose gain has not been spent yet.
fn runs_at(
    placed: &[PlacedJob<'_>],
    which: &[usize],
    depth: usize,
    rate: u32,
    offline: bool,
) -> Vec<MixRun> {
    let mut out: Vec<MixRun> = Vec::new();
    // The buses already summed, so the rest of a bus's jobs are passed over
    // rather than summed again. Two Precomp layers at the same depth are two
    // buses, which is why the chain itself is the mark and not the depth.
    let mut summed: Vec<(usize, *const AudioChain)> = Vec::new();
    for &i in which {
        let job = placed[i].job;
        let carriers = &job.carriers[depth.min(job.carriers.len())..];
        let Some(at) = bus_at(job, depth) else {
            let (gain, envelope) = gain_bake(
                Some(job),
                carriers,
                placed[i].start_frame,
                placed[i].samples.len() / 2,
                rate,
            );
            out.push(MixRun {
                first: i,
                layer: job.layer,
                start_frame: placed[i].start_frame,
                of: RunOf::Job(i),
                gain,
                envelope,
            });
            continue;
        };
        let Some(chain) = job.carriers[at].chain.as_ref() else {
            continue;
        };
        if summed.contains(&(at, std::sync::Arc::as_ptr(chain))) {
            continue;
        }
        summed.push((at, std::sync::Arc::as_ptr(chain)));
        let group: Vec<usize> = which
            .iter()
            .copied()
            .filter(|&j| {
                bus_at(placed[j].job, depth) == Some(at)
                    && placed[j].job.carriers[at]
                        .chain
                        .as_ref()
                        .is_some_and(|c| std::sync::Arc::ptr_eq(c, chain))
            })
            .collect();
        let inner = runs_at(placed, &group, at + 1, rate, offline);
        // The sum spans from the earliest run to the latest, which may reach
        // before the bus's own span: a rack inside it has already placed its
        // sound early to answer for its latency.
        let span = inner.iter().map(|r| {
            let samples: &[f32] = match &r.of {
                RunOf::Job(j) => placed[*j].samples,
                RunOf::Bus(s) => s,
            };
            (r.start_frame, r.start_frame + (samples.len() / 2) as i64)
        });
        let (Some(first), Some(last)) = (
            span.clone().map(|(a, _)| a).min(),
            span.map(|(_, b)| b).max(),
        ) else {
            continue;
        };
        if last <= first {
            continue;
        }
        let sources: Vec<lumit_audio::mix::PlacedAudio<'_>> = inner
            .iter()
            .map(|r| lumit_audio::mix::PlacedAudio {
                start_frame: r.start_frame - first,
                samples: match &r.of {
                    RunOf::Job(j) => placed[*j].samples,
                    RunOf::Bus(s) => s,
                },
                gain: r.gain,
                envelope: r.envelope.clone(),
            })
            .collect();
        // No master fader and no ceiling on a bus: the limiter is the master's
        // own last stage and this sum is in the middle of the desk.
        let sum = lumit_audio::mix::sum_stereo(&sources, (last - first) as usize);
        let (samples, start_frame) = match chain_bake(chain, &sum, first, rate, offline) {
            // Placed the rack's latency earlier, exactly as a job's own rack
            // is, so the processed bus lands where the dry sum did.
            Some((wet, latency)) => (wet, first - i64::from(latency)),
            None => (sum, first),
        };
        let (gain, envelope) = gain_bake(
            None,
            &job.carriers[depth..=at],
            start_frame,
            samples.len() / 2,
            rate,
        );
        out.push(MixRun {
            first: i,
            layer: job.layer,
            start_frame,
            of: RunOf::Bus(samples),
            gain,
            envelope,
        });
    }
    out
}

/// Delivery presets (docs/06-RENDER-PIPELINE.md §7.5): frame, codec, and
/// bitrates as data, not code. Custom keeps the comp's own size and the
/// dialogue's choices; it is also the default (Settings → Export),
/// matching the implicit behaviour every "Export…" action had before that
/// setting existed.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum ExportPreset {
    #[default]
    Custom,
    Youtube1080p60,
    Youtube1440p60,
    Youtube4k60,
    Vertical1080p60,
}

/// The parameter row one preset stamps into the export dialogue.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PresetParams {
    pub size: (u32, u32),
    pub codec: lumit_media::encode::VideoCodec,
    /// VBR average target, bits/second.
    pub target_bps: i64,
    /// VBR peak, bits/second.
    pub peak_bps: i64,
}

/// Audio on all delivery presets: AAC 320 kbps, 48 kHz (docs/06 §7.5).
pub const PRESET_AUDIO_BPS: i64 = 320_000;
/// Export audio sample rate (docs/06 §7.5: 48 kHz on delivery presets).
pub const EXPORT_AUDIO_RATE: u32 = 48_000;

impl ExportPreset {
    pub const ALL: [ExportPreset; 5] = [
        ExportPreset::Custom,
        ExportPreset::Youtube1080p60,
        ExportPreset::Youtube1440p60,
        ExportPreset::Youtube4k60,
        ExportPreset::Vertical1080p60,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ExportPreset::Custom => "Custom (comp size)",
            ExportPreset::Youtube1080p60 => "YouTube 1080p60",
            ExportPreset::Youtube1440p60 => "YouTube 1440p60",
            ExportPreset::Youtube4k60 => "YouTube 4K60",
            ExportPreset::Vertical1080p60 => "Vertical 1080×1920p60",
        }
    }

    /// The parameters this preset stamps; None for Custom (the dialogue's
    /// own fields apply).
    pub fn params(self) -> Option<PresetParams> {
        use lumit_media::encode::VideoCodec;
        match self {
            ExportPreset::Custom => None,
            // H.264 high, VBR 16 target / 24 peak (docs/06 §7.5).
            ExportPreset::Youtube1080p60 => Some(PresetParams {
                size: (1920, 1080),
                codec: VideoCodec::H264,
                target_bps: 16_000_000,
                peak_bps: 24_000_000,
            }),
            // HEVC (H.264 fallback), VBR 25 target / 35 peak — YouTube's
            // 1440p60 band (docs/06 §7.5).
            ExportPreset::Youtube1440p60 => Some(PresetParams {
                size: (2560, 1440),
                codec: VideoCodec::Hevc,
                target_bps: 25_000_000,
                peak_bps: 35_000_000,
            }),
            // HEVC (the ladder falls back to x265 when no hardware offers
            // it), VBR 45 target / 60 peak — YouTube's 2160p60 band.
            ExportPreset::Youtube4k60 => Some(PresetParams {
                size: (3840, 2160),
                codec: VideoCodec::Hevc,
                target_bps: 45_000_000,
                peak_bps: 60_000_000,
            }),
            // The vertical variant of the 1080p60 preset (docs/06 §7.5).
            ExportPreset::Vertical1080p60 => Some(PresetParams {
                size: (1080, 1920),
                codec: VideoCodec::H264,
                target_bps: 16_000_000,
                peak_bps: 24_000_000,
            }),
        }
    }

    /// Suggested file name for the save dialogue.
    pub fn default_file_name(self) -> &'static str {
        match self {
            ExportPreset::Custom => "export.mp4",
            ExportPreset::Youtube1080p60 => "youtube-1080p60.mp4",
            ExportPreset::Youtube1440p60 => "youtube-1440p60.mp4",
            ExportPreset::Youtube4k60 => "youtube-4k60.mp4",
            ExportPreset::Vertical1080p60 => "vertical-1080x1920.mp4",
        }
    }
}

/// The sample rates an export can write. Three, and only three: the CD rate,
/// the delivery rate every preset uses, and the high-resolution master rate.
/// A rate outside this list is refused rather than nudged to the nearest one
/// — an export that quietly wrote 48 kHz when 44.1 was asked for would be a
/// file that is not what it says it is.
pub const EXPORT_AUDIO_RATES: &[u32] = &[44_100, EXPORT_AUDIO_RATE, 96_000];

/// How many bits one written sample carries.
///
/// In plain terms: this is the *file's* resolution, not the mix's. Lumit mixes
/// in 32-bit floats whatever is chosen here; the depth decides how finely that
/// mix is written down. It means something only for the uncompressed forms —
/// a lossy codec stores coefficients, not samples, and has no sample width at
/// all (see [`FormatCaps::audio_depths`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum AudioDepth {
    /// Sixteen bits a sample — CD resolution, and what every delivery file
    /// has always carried.
    #[default]
    Sixteen,
    /// Twenty-four bits a sample — the master resolution, where the extra
    /// headroom is wanted for further work.
    TwentyFour,
}

impl AudioDepth {
    pub const ALL: [AudioDepth; 2] = [AudioDepth::Sixteen, AudioDepth::TwentyFour];

    pub fn bits(self) -> u32 {
        match self {
            AudioDepth::Sixteen => 16,
            AudioDepth::TwentyFour => 24,
        }
    }
}

/// How many channels the written file carries.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum AudioLayout {
    /// One channel: the comp's stereo mix folded down (see
    /// [`lumit_audio::mix::downmix_to_mono`], which states the law).
    Mono,
    /// Two channels — the comp's mix as it is mixed and as it is played back.
    #[default]
    Stereo,
}

impl AudioLayout {
    pub const ALL: [AudioLayout; 2] = [AudioLayout::Mono, AudioLayout::Stereo];

    /// The interleave width every buffer downstream of the fold-down uses.
    pub fn channels(self) -> u16 {
        match self {
            AudioLayout::Mono => 1,
            AudioLayout::Stereo => 2,
        }
    }
}

/// The sound-only containers an export can write (docs/06 §7.4).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum AudioFormat {
    /// AAC in an `.m4a` — the delivery form, same codec a video export uses.
    #[default]
    M4a,
    /// Uncompressed 16-bit PCM in a `.wav` — the master form.
    Wav,
}

impl AudioFormat {
    pub fn label(self) -> &'static str {
        match self {
            AudioFormat::M4a => "M4A (AAC)",
            AudioFormat::Wav => "WAV (uncompressed)",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            AudioFormat::M4a => "m4a",
            AudioFormat::Wav => "wav",
        }
    }

    /// The codec this container is written with. AAC is AAC whatever depth
    /// was asked for — it stores no samples to give a width to — so the depth
    /// only picks between the two PCM widths, and a depth AAC cannot honour
    /// is refused by [`ExportSpec::check`] long before this is called.
    pub fn codec(self, depth: AudioDepth) -> lumit_media::encode::AudioCodec {
        match (self, depth) {
            (AudioFormat::M4a, _) => lumit_media::encode::AudioCodec::Aac,
            (AudioFormat::Wav, AudioDepth::Sixteen) => lumit_media::encode::AudioCodec::PcmS16,
            (AudioFormat::Wav, AudioDepth::TwentyFour) => lumit_media::encode::AudioCodec::PcmS24,
        }
    }
}

/// What the export writes: a video file, one still image per frame, or
/// sound with no picture at all.
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum ExportFormat {
    /// An `.mp4`, in the given codec.
    Video(lumit_media::encode::VideoCodec),
    /// A numbered image per frame — `shot.00001.png` beside the chosen path.
    Images(lumit_media::encode::ImageFormat),
    /// An `.m4a` or `.wav` of the comp's mix, with no video stream.
    Audio(AudioFormat),
}

/// What one output format can and cannot carry (docs/06 §7.4).
///
/// In plain terms: the export dialog draws every option, but not every option
/// means anything in every file. A `.png` has no bitrate; an `.mp4` cannot hold
/// an alpha channel; a `.wav` has no picture to set a depth on. This table says
/// which is which, in one place, so the dialog and the exporter cannot disagree
/// — and so a setting that a format cannot honour is refused rather than
/// quietly ignored.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FormatCaps {
    /// Carries a picture at all.
    pub video: bool,
    /// Can carry the comp's sound.
    pub audio: bool,
    /// Can carry an alpha channel (so the Channels choice means something).
    pub alpha: bool,
    /// The colour depths this format can write, best last.
    pub depths: &'static [lumit_media::encode::BitDepth],
    /// The sample rates this format's audio stream can be written at, empty
    /// where it carries no sound at all.
    pub audio_rates: &'static [u32],
    /// The sample widths this format's audio stream can be written at, empty
    /// where it carries no sound at all.
    ///
    /// AAC lists **sixteen only**, and that is not a claim about AAC's
    /// precision: a lossy transform codec stores coefficients rather than
    /// samples, so it has no sample width to set. Asking an `.mp4` or an
    /// `.m4a` for twenty-four bits is therefore answered honestly — the
    /// format cannot carry the choice — rather than by accepting the setting
    /// and writing the same file either way.
    pub audio_depths: &'static [AudioDepth],
    /// A video bitrate applies (lossless formats have none to choose).
    pub bit_rate: bool,
    /// The container holds metadata.
    pub metadata: bool,
    /// The colour spaces this format's container can *state* — the nclx/`colr`
    /// box on an mp4, the cICP chunk on a PNG. A space the file cannot name is
    /// refused rather than written unlabelled, because a wide-gamut file that
    /// says nothing is read as sRGB and comes back looking wrong. Empty where
    /// the format carries no picture at all.
    pub colour_spaces: &'static [ColourSpace],
}

use lumit_media::encode::BitDepth;

/// Eight bits only — every video codec Lumit writes in v1 (docs/06 §7.4:
/// ProRes and DNxHR, which is where 4444 and deeper live, are not in v1).
const EIGHT_ONLY: &[BitDepth] = &[BitDepth::Eight];
/// The still formats carry either width.
const EIGHT_OR_SIXTEEN: &[BitDepth] = &[BitDepth::Eight, BitDepth::Sixteen];
/// OpenEXR carries floats and nothing else — writing eight-bit codes into one
/// would be a scene file holding a picture, which is the one thing the format
/// is for not doing. Half first because it is what a render farm writes.
const EXR_DEPTHS: &[BitDepth] = &[BitDepth::Half, BitDepth::Float];
/// AAC has no sample width of its own; only the delivery default stands.
const AAC_DEPTH: &[AudioDepth] = &[AudioDepth::Sixteen];
/// Uncompressed PCM in a `.wav` carries either width.
const PCM_DEPTHS: &[AudioDepth] = &[AudioDepth::Sixteen, AudioDepth::TwentyFour];
/// What a still sequence can state. PNG carries cICP and TIFF carries nothing,
/// and one export writes one kind of file, so the honest common answer is the
/// space that needs no tag; `a_format_refuses_a_colour_space_it_cannot_state`
/// keeps this row and the exporter in step.
const STILL_COLOUR_SPACES: &[ColourSpace] = UNTAGGED_COLOUR_SPACE;

impl ExportFormat {
    /// This format's capability row.
    pub fn caps(self) -> FormatCaps {
        match self {
            // H.264/HEVC in mp4: 4:2:0, eight bits, no alpha, a bitrate to
            // choose, and a container that holds metadata.
            ExportFormat::Video(_) => FormatCaps {
                video: true,
                audio: true,
                alpha: false,
                depths: EIGHT_ONLY,
                audio_rates: EXPORT_AUDIO_RATES,
                audio_depths: AAC_DEPTH,
                bit_rate: true,
                metadata: true,
                colour_spaces: BUILT_IN_COLOUR_SPACES,
            },
            // Stills: lossless RGBA, either depth, no sound and no bitrate.
            // The image2 muxer writes one file per frame and has nowhere to
            // put container metadata.
            ExportFormat::Images(f) => FormatCaps {
                video: true,
                audio: false,
                alpha: true,
                depths: if f == lumit_media::encode::ImageFormat::Exr {
                    EXR_DEPTHS
                } else {
                    EIGHT_OR_SIXTEEN
                },
                audio_rates: &[],
                audio_depths: &[],
                bit_rate: false,
                metadata: false,
                colour_spaces: STILL_COLOUR_SPACES,
            },
            ExportFormat::Audio(f) => FormatCaps {
                video: false,
                audio: true,
                alpha: false,
                depths: &[],
                audio_rates: EXPORT_AUDIO_RATES,
                audio_depths: match f {
                    AudioFormat::M4a => AAC_DEPTH,
                    AudioFormat::Wav => PCM_DEPTHS,
                },
                // AAC has a bitrate; PCM is exactly what it is.
                bit_rate: f == AudioFormat::M4a,
                metadata: true,
                colour_spaces: &[],
            },
        }
    }

    /// The file extension this format writes.
    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Video(_) => "mp4",
            ExportFormat::Images(f) => f.extension(),
            ExportFormat::Audio(f) => f.extension(),
        }
    }
}

/// Which channels the written file carries.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum Channels {
    /// Colour only — the alpha channel is written opaque, so what a viewer
    /// sees is the comp over its own background.
    #[default]
    Rgb,
    /// Colour and the composite's own coverage, for a file that will be
    /// layered over something else.
    RgbAlpha,
}

/// How the colour channels relate to the alpha channel in the written file
/// (docs/06 §3.4). The compositor works premultiplied throughout, so
/// premultiplied is a pass-through and straight is a division.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum AlphaMode {
    /// Colour already multiplied by coverage — the working form, and what
    /// most compositors expect back.
    #[default]
    Premultiplied,
    /// Colour un-multiplied, full-strength wherever there is any coverage at
    /// all. What paint programs and some delivery specs ask for.
    Straight,
}

/// The colour space the file is written in — the export's final transform
/// (docs/06 §7.4).
///
/// In plain terms: the compositor works in scene-linear light and hands the
/// export a frame already encoded for a normal screen (sRGB primaries, sRGB
/// curve — what the Viewer shows). Something has to say what a *delivered*
/// file contains, and this is it: which three primary colours the numbers are
/// mixtures of, and what curve maps a number to an amount of light. The
/// transform runs at the pack stage, and the container is stamped so the file
/// says what it is rather than leaving the player to guess.
///
/// Every built-in space is D65-white, so converting between them is one 3×3
/// matrix and one curve — no white-point adaptation is involved.
#[derive(Clone, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum ColourSpace {
    /// sRGB — Rec.709 primaries with the sRGB transfer curve (IEC 61966-2-1).
    /// The Viewer's own encode, so this is a pass-through: the frame
    /// arrives in it and is written untouched. The default, and what every
    /// Lumit export before the family existed wrote.
    #[default]
    SrgbRec709,
    /// Rec.709 primaries, **no** transfer curve: the code values are linear
    /// light. For a file going straight back into a compositor, where a
    /// display curve is something to undo.
    Linear,
    /// Rec.709 proper — ITU-R BT.709-6 primaries and its opto-electronic
    /// transfer function. BT.1886 is the display half of the same pair (a
    /// 2.4-gamma EOTF); a *file* carries the OETF, which is what is applied.
    Rec709,
    /// Rec.2020 — ITU-R BT.2020-2 wide primaries and its transfer function,
    /// for a wide-gamut delivery.
    Rec2020,
    /// Display P3 — the DCI-P3 primaries on a D65 white with the sRGB curve
    /// (SMPTE EG 432-1 primaries, IEC 61966-2-1 curve): what Apple's displays
    /// and the wide-gamut web want.
    DisplayP3,
    /// A named output space from an OCIO config (docs/06 §2, post-v1). Kept in
    /// the model so a project written today names its space the same way it
    /// will then; an export that asks for one before OCIO exists is refused,
    /// because a wrong colour space in a delivered file is worse than an
    /// export that did not run.
    Ocio(String),
}

/// Every built-in space, in the order the export drawing lists them. A format
/// whose container can state its colour carries this whole set.
pub const BUILT_IN_COLOUR_SPACES: &[ColourSpace] = &[
    ColourSpace::SrgbRec709,
    ColourSpace::Linear,
    ColourSpace::Rec709,
    ColourSpace::Rec2020,
    ColourSpace::DisplayP3,
];

/// The one space that needs no container tag, because it is what an untagged
/// file is universally taken to be. A format that cannot state its colour can
/// still write this one honestly, and is refused any other.
pub const UNTAGGED_COLOUR_SPACE: &[ColourSpace] = &[ColourSpace::SrgbRec709];

impl ColourSpace {
    /// Whether this build can perform the transform **with no project in
    /// hand**. Every built-in space can; a config's space depends on the
    /// project's config, so it is asked about separately — see
    /// [`ExportSpec::check_with_colour`].
    pub fn is_available(&self) -> bool {
        !matches!(self, ColourSpace::Ocio(_))
    }

    /// The config's name, if this is one of its spaces.
    #[must_use]
    pub fn ocio_name(&self) -> Option<&str> {
        match self {
            ColourSpace::Ocio(name) => Some(name),
            _ => None,
        }
    }

    pub fn label(&self) -> String {
        match self {
            ColourSpace::SrgbRec709 => "Rec. 709 (sRGB)".to_owned(),
            ColourSpace::Linear => "Linear".to_owned(),
            ColourSpace::Rec709 => "Rec. 709".to_owned(),
            ColourSpace::Rec2020 => "Rec. 2020".to_owned(),
            ColourSpace::DisplayP3 => "Display P3".to_owned(),
            ColourSpace::Ocio(name) => name.clone(),
        }
    }

    /// The stable name a stored preset and the seam carry. The default space
    /// is the empty string, so a preset written before the family existed
    /// still loads as itself; every other built-in has a short lower-case key
    /// that will not move when its user-facing label is reworded, and an OCIO
    /// space carries its own name from the config.
    pub fn stored_name(&self) -> String {
        match self {
            ColourSpace::SrgbRec709 => String::new(),
            ColourSpace::Linear => "linear".to_owned(),
            ColourSpace::Rec709 => "rec709".to_owned(),
            ColourSpace::Rec2020 => "rec2020".to_owned(),
            ColourSpace::DisplayP3 => "display-p3".to_owned(),
            ColourSpace::Ocio(name) => name.clone(),
        }
    }

    /// The inverse of [`Self::stored_name`]. An unrecognised name is an OCIO
    /// space — which `check` then refuses — rather than a silent fall back to
    /// the default: a file delivered in the wrong space is worse than an
    /// export that did not run.
    pub fn from_stored_name(name: &str) -> Self {
        match name {
            "" => ColourSpace::SrgbRec709,
            "linear" => ColourSpace::Linear,
            "rec709" => ColourSpace::Rec709,
            "rec2020" => ColourSpace::Rec2020,
            "display-p3" => ColourSpace::DisplayP3,
            other => ColourSpace::Ocio(other.to_owned()),
        }
    }

    /// What the container is stamped with, so the file states its own colour.
    pub fn tags(&self) -> lumit_media::encode::ColourTags {
        use lumit_media::encode::ColourTags;
        match self {
            ColourSpace::SrgbRec709 => ColourTags::Srgb,
            ColourSpace::Linear => ColourTags::Linear,
            ColourSpace::Rec709 => ColourTags::Bt709,
            ColourSpace::Rec2020 => ColourTags::Bt2020,
            ColourSpace::DisplayP3 => ColourTags::DisplayP3,
            // **Untagged, deliberately** (docs/impl/ocio.md §5.2). A
            // config's name has no reliable primaries or transfer metadata in
            // general — the config author may have composed anything — so a
            // file written through one carries no colour tag rather than a
            // guessed one. A player that finds no tag falls back to its own
            // sensible default; a player that finds a wrong tag confidently
            // shows the wrong colour, which is worse. The known ACES
            // display/view names that correspond exactly to a built-in tag may
            // reuse it one explicit table entry at a time; none does yet.
            ColourSpace::Ocio(_) => ColourTags::Unspecified,
        }
    }

    /// The per-pixel transform the pack stage applies, or `None` when the
    /// frame already *is* this space and nothing should touch it.
    pub fn transform(&self) -> Option<ColourTransform> {
        let (primaries, transfer) = match self {
            // The frame arrives in this space. No arithmetic at all — an
            // identity that ran anyway would still round twice.
            //
            // An OCIO space takes this arm for a different reason and it is the
            // load-bearing one: its transform ran on the graphics card, in the
            // same display blit the Viewer presents through (§5.2). A second
            // transform here would be a second implementation of one transform
            // in the delivery path, which is the exact structure the
            // preview-equals-export promise forbids.
            ColourSpace::SrgbRec709 | ColourSpace::Ocio(_) => return None,
            ColourSpace::Linear => (None, Transfer::Linear),
            ColourSpace::Rec709 => (None, Transfer::Bt709),
            ColourSpace::Rec2020 => (Some(REC2020_PRIMARIES), Transfer::Bt2020),
            ColourSpace::DisplayP3 => (Some(DISPLAY_P3_PRIMARIES), Transfer::Srgb),
        };
        Some(ColourTransform {
            matrix: primaries.map(|p| primaries_change(&REC709_PRIMARIES, &p)),
            transfer,
        })
    }
}

// ---------------------------------------------------------------------------
// The built-in colour transforms.
//
// In plain terms: a colour space is two things — which three lights the three
// numbers stand for (the *primaries*, given as CIE 1931 xy chromaticities plus
// a white point), and what curve turns a number into an amount of light (the
// *transfer function*). Converting between two spaces is therefore: undo the
// source curve to get linear light, change the primaries with a 3×3 matrix,
// then apply the destination curve. The matrix is derived from the published
// chromaticities rather than typed out, so there are no transcribed digits to
// get wrong, and `rec709_matrix_matches_the_published_one` checks the
// derivation against BT.709's own printed matrix.
// ---------------------------------------------------------------------------

/// CIE 1931 xy chromaticities: red, green, blue, white.
type Primaries = [[f64; 2]; 4];

/// ITU-R BT.709-6, Table 1. Also IEC 61966-2-1's sRGB primaries — sRGB and
/// Rec.709 share them; only the transfer curve differs.
const REC709_PRIMARIES: Primaries = [
    [0.640, 0.330],
    [0.300, 0.600],
    [0.150, 0.060],
    [0.3127, 0.3290], // D65
];

/// ITU-R BT.2020-2, Table 1.
const REC2020_PRIMARIES: Primaries = [
    [0.708, 0.292],
    [0.170, 0.797],
    [0.131, 0.046],
    [0.3127, 0.3290], // D65
];

/// SMPTE EG 432-1 / RP 431-2 P3 primaries on a D65 white — "Display P3".
const DISPLAY_P3_PRIMARIES: Primaries = [
    [0.680, 0.320],
    [0.265, 0.690],
    [0.150, 0.060],
    [0.3127, 0.3290], // D65
];

/// The transfer function a space encodes its linear light with.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Transfer {
    /// None at all: the code value *is* the light.
    Linear,
    /// IEC 61966-2-1 (sRGB).
    Srgb,
    /// ITU-R BT.709-6 §1.2 opto-electronic transfer function.
    Bt709,
    /// ITU-R BT.2020-2, Table 4.
    Bt2020,
}

impl Transfer {
    /// Linear light in 0..1 to an encoded code value in 0..1.
    fn encode(self, v: f64) -> f64 {
        let v = v.clamp(0.0, 1.0);
        match self {
            Transfer::Linear => v,
            // IEC 61966-2-1: 12.92*L below the knee, 1.055*L^(1/2.4) — 0.055
            // above it.
            Transfer::Srgb => {
                if v <= 0.003_130_8 {
                    12.92 * v
                } else {
                    1.055 * v.powf(1.0 / 2.4) - 0.055
                }
            }
            // BT.709-6 §1.2: 4.5*L below 0.018, 1.099*L^0.45 — 0.099 above.
            Transfer::Bt709 => {
                if v < 0.018 {
                    4.5 * v
                } else {
                    1.099 * v.powf(0.45) - 0.099
                }
            }
            // BT.2020-2 Table 4: the same shape with the constants carried to
            // the precision the ten- and twelve-bit systems need.
            Transfer::Bt2020 => {
                const A: f64 = 1.099_296_826_809_442;
                const B: f64 = 0.018_053_968_510_807;
                if v < B {
                    4.5 * v
                } else {
                    A * v.powf(0.45) - (A - 1.0)
                }
            }
        }
    }
}

/// Decode one sRGB code value (0..1) to linear light: IEC 61966-2-1's inverse,
/// in `f64`. [`lumit_core::pixels::srgb_decode`] is the `f32`/byte twin; the
/// export wants the wider type because a sixteen-bit frame has far more codes
/// than a byte does.
fn srgb_to_linear(v: f64) -> f64 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.040_45 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// The RGB→XYZ matrix for one set of primaries (SMPTE RP 177-1993 §3.3):
/// scale each primary's chromaticity vector so the three together sum to the
/// white point at Y = 1.
fn rgb_to_xyz(p: &Primaries) -> [[f64; 3]; 3] {
    // Each primary as an unscaled XYZ direction (Y = 1).
    let dir = |xy: [f64; 2]| [xy[0] / xy[1], 1.0, (1.0 - xy[0] - xy[1]) / xy[1]];
    let (r, g, b, w) = (dir(p[0]), dir(p[1]), dir(p[2]), dir(p[3]));
    // Columns r, g, b; solve M.s = w for the three scale factors.
    let m = [[r[0], g[0], b[0]], [r[1], g[1], b[1]], [r[2], g[2], b[2]]];
    let s = mul3(&invert3(&m), w);
    [
        [r[0] * s[0], g[0] * s[1], b[0] * s[2]],
        [r[1] * s[0], g[1] * s[1], b[1] * s[2]],
        [r[2] * s[0], g[2] * s[1], b[2] * s[2]],
    ]
}

/// The linear-light matrix taking RGB in `from`'s primaries to RGB in `to`'s:
/// through XYZ and back. Both are D65, so no chromatic adaptation is involved.
fn primaries_change(from: &Primaries, to: &Primaries) -> [[f64; 3]; 3] {
    let a = rgb_to_xyz(from);
    let b = invert3(&rgb_to_xyz(to));
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = (0..3).map(|k| b[i][k] * a[k][j]).sum();
        }
    }
    out
}

/// 3×3 matrix times a column vector.
fn mul3(m: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

/// 3×3 inverse by the adjugate. A primaries matrix is never singular — three
/// distinct chromaticities are linearly independent — so a zero determinant
/// answers the identity rather than dividing by nothing.
fn invert3(m: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let c = |a: usize, b: usize, c2: usize, d: usize| m[a][b] * m[c2][d];
    let a00 = c(1, 1, 2, 2) - c(1, 2, 2, 1);
    let a01 = c(0, 2, 2, 1) - c(0, 1, 2, 2);
    let a02 = c(0, 1, 1, 2) - c(0, 2, 1, 1);
    let det = m[0][0] * a00 + m[1][0] * a01 + m[2][0] * a02;
    if det.abs() < f64::EPSILON {
        return [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    }
    let a10 = c(1, 2, 2, 0) - c(1, 0, 2, 2);
    let a11 = c(0, 0, 2, 2) - c(0, 2, 2, 0);
    let a12 = c(0, 2, 1, 0) - c(0, 0, 1, 2);
    let a20 = c(1, 0, 2, 1) - c(1, 1, 2, 0);
    let a21 = c(0, 1, 2, 0) - c(0, 0, 2, 1);
    let a22 = c(0, 0, 1, 1) - c(0, 1, 1, 0);
    [
        [a00 / det, a01 / det, a02 / det],
        [a10 / det, a11 / det, a12 / det],
        [a20 / det, a21 / det, a22 / det],
    ]
}

/// One export's colour transform, worked out once and applied per pixel.
///
/// In plain terms: the frame arrives sRGB-encoded. Undo that curve to get
/// linear light, optionally move to different primaries, then apply the
/// destination's curve. Deterministic — plain `f64` arithmetic in a fixed
/// order, no threading and no graphics card.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ColourTransform {
    /// Linear-light primaries change, or `None` when the destination shares
    /// Rec.709's primaries and only the curve differs.
    matrix: Option<[[f64; 3]; 3]>,
    transfer: Transfer,
}

impl ColourTransform {
    /// One straight (un-multiplied) sRGB-encoded RGB triple in 0..1 to the
    /// destination space's encoded values, also in 0..1.
    #[must_use]
    pub fn apply(&self, rgb: [f64; 3]) -> [f64; 3] {
        let lin = [
            srgb_to_linear(rgb[0]),
            srgb_to_linear(rgb[1]),
            srgb_to_linear(rgb[2]),
        ];
        // A wider destination gamut cannot lose a colour; a narrower one can be
        // asked for one it has no mixture of, and the encode clamps — the
        // ordinary out-of-gamut answer for a display-referred file.
        let lin = match &self.matrix {
            Some(m) => mul3(m, lin),
            None => lin,
        };
        [
            self.transfer.encode(lin[0]),
            self.transfer.encode(lin[1]),
            self.transfer.encode(lin[2]),
        ]
    }
}

/// A crop applied on the way out, as pixel insets from each edge of the
/// composition (distances are pixels at composition size, never a
/// percentage). `Crop::NONE` is no crop.
///
/// In plain terms: the four numbers are how much to take off the top, the
/// left, the bottom and the right — the reading the export drawing shows as
/// `T · L · B · R`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Crop {
    pub top: u32,
    pub left: u32,
    pub bottom: u32,
    pub right: u32,
}

impl Crop {
    pub const NONE: Crop = Crop {
        top: 0,
        left: 0,
        bottom: 0,
        right: 0,
    };

    pub fn is_none(self) -> bool {
        self == Crop::NONE
    }

    /// The size a `w`×`h` frame becomes. Insets that meet or cross are
    /// clamped so at least one pixel survives — a crop that asked for nothing
    /// is a slip of the fingers, not a reason to fail an export.
    pub fn output_size(self, w: u32, h: u32) -> (u32, u32) {
        let out_w = w
            .saturating_sub(self.left.saturating_add(self.right))
            .max(1);
        let out_h = h
            .saturating_sub(self.top.saturating_add(self.bottom))
            .max(1);
        (out_w.min(w.max(1)), out_h.min(h.max(1)))
    }

    /// The window this crop keeps, as `(x, y, width, height)` in source
    /// pixels — clamped to the frame the same way [`Self::output_size`] is, so
    /// the two can never disagree about which pixels are copied.
    pub fn window(self, w: u32, h: u32) -> (u32, u32, u32, u32) {
        let (out_w, out_h) = self.output_size(w, h);
        let x = self.left.min(w.saturating_sub(out_w));
        let y = self.top.min(h.saturating_sub(out_h));
        (x, y, out_w, out_h)
    }

    /// Copy the kept window out of a tightly-packed frame of `bytes_per_px`.
    /// A buffer too small for the frame it claims to be comes back unchanged,
    /// which is the calm answer: no panic, and a caller bug shows as a
    /// full-size frame rather than a crash mid-export.
    ///
    /// `per_px` counts *elements* of `T` in a pixel — four bytes for an
    /// eight-bit frame, four codes for a sixteen-bit one — so one row copy
    /// serves both depths.
    pub fn apply<T: Copy>(self, frame: &[T], w: u32, h: u32, per_px: usize) -> Vec<T> {
        let (x, y, out_w, out_h) = self.window(w, h);
        let src_row = (w as usize).saturating_mul(per_px);
        // Nothing to crop, an empty frame, or a buffer smaller than the frame
        // it claims to be: hand it back whole. No panics in engine crates
        // (docs/14 §4), and a caller bug must show as a full-size frame rather
        // than a crash halfway through an export.
        if self.is_none() || w == 0 || h == 0 || frame.len() < src_row.saturating_mul(h as usize) {
            return frame.to_vec();
        }
        let dst_row = (out_w as usize) * per_px;
        let skip = (x as usize) * per_px;
        let mut out = Vec::with_capacity(dst_row * out_h as usize);
        for row in 0..out_h as usize {
            let start = (y as usize + row) * src_row + skip;
            out.extend_from_slice(&frame[start..start + dst_row]);
        }
        out
    }

    /// The crop equivalent to the Viewer's region of interest — the rectangle
    /// the user swept on the picture, which crosses every boundary as
    /// fractions `[x0, y0, x1, y1]` rather than pixels (which pixel a
    /// point is depends on the raster, and the raster changes with the preview
    /// resolution).
    ///
    /// Degenerate input answers no crop, exactly as a degenerate region clears
    /// the region: a drag that ended where it began is a gesture, not an
    /// error.
    pub fn from_region(region: [f64; 4], w: u32, h: u32) -> Crop {
        let [x0, y0, x1, y1] = region;
        if !region.iter().all(|v| v.is_finite()) || x1 <= x0 || y1 <= y0 {
            return Crop::NONE;
        }
        let px = |v: f64, size: u32| (v.clamp(0.0, 1.0) * f64::from(size)).round() as u32;
        let (l, t, r, b) = (px(x0, w), px(y0, h), px(x1, w), px(y1, h));
        Crop {
            top: t,
            left: l,
            bottom: h.saturating_sub(b),
            right: w.saturating_sub(r),
        }
    }
}

/// The video bitrate, chosen or worked out (docs/06 §7.5).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum Bitrate {
    /// Work a high default out from the resolution and the rate — what the
    /// dialog's *Auto* face means.
    #[default]
    Auto,
    /// Set no bitrate at all and let the encoder pick its own quality. What a
    /// blank bitrate field has always meant, kept as its own answer
    /// rather than folded into `Auto`, because the two produce different
    /// files and a preset saved under one must not silently become the other.
    EncoderDefault,
    /// The number the user typed, in bits per second, with an optional VBR
    /// peak (None takes the 1.5× fallback the resolver has always used).
    Manual {
        target_bps: i64,
        peak_bps: Option<i64>,
    },
}

/// Bits per pixel per second a codec needs for a delivery-quality picture —
/// the constants behind [`Bitrate::Auto`]. Taken from the preset table
/// (docs/06 §7.5): 1920×1080 at 60 wants 16 Mbps of H.264, which is 0.13 bits
/// per pixel, and HEVC buys roughly a quarter off that.
const H264_BITS_PER_PIXEL: f64 = 0.13;
const HEVC_BITS_PER_PIXEL: f64 = 0.10;

/// The VBR peak as a multiple of the target, when no peak was given — the
/// same 1.5× the spec resolver has always fallen back to.
pub const PEAK_MULTIPLE: f64 = 1.5;

/// A high-quality bitrate for `w`×`h` at `fps`, as `(target, peak)` in bits
/// per second, rounded to a whole megabit and clamped to something a file can
/// actually hold.
///
/// In plain terms: more pixels and more frames need more bits, in proportion.
/// This is deliberately a straight line rather than a curve fitted to the
/// preset table — a preset stamps its own exact numbers, and *Auto* only has
/// to be a good default for a size no preset covers.
pub fn auto_bitrate(
    w: u32,
    h: u32,
    fps: f64,
    codec: lumit_media::encode::VideoCodec,
) -> (i64, i64) {
    let bits_per_px = match codec {
        lumit_media::encode::VideoCodec::H264 => H264_BITS_PER_PIXEL,
        lumit_media::encode::VideoCodec::Hevc => HEVC_BITS_PER_PIXEL,
    };
    let pixels_per_second = f64::from(w) * f64::from(h) * fps.clamp(1.0, 1000.0);
    let mbps = (pixels_per_second * bits_per_px / 1e6)
        .round()
        .clamp(1.0, 400.0);
    let target = (mbps as i64) * 1_000_000;
    let peak = ((target as f64) * PEAK_MULTIPLE).round() as i64;
    (target, peak)
}

/// Whether the export reads and writes the disk frame cache while it runs.
///
/// In plain terms: the cache of already-rendered frames on disk speeds up
/// scrubbing, but an export is a single pass through the timeline — it would
/// fill the cache with frames nobody is going to ask for again, evicting the
/// ones the user *is* working with. Off is the honest default.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum DiskCachePolicy {
    /// Neither read nor written — the export renderer's own state today.
    #[default]
    Off,
    /// Read frames already banked, but bank nothing new.
    ReadOnly,
}

/// The export's answer for motion blur (docs/15 §12A.4, the Time section's
/// first row). Blur passes two gates — the composition's master switch and
/// each layer's own switch (docs/06 §4) — so the three answers are the
/// three useful things to say about the master while the checks stand.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum MotionBlurOverride {
    /// *Current settings*: every composition's master and every layer's switch
    /// stand exactly as saved. The default, so a spec written before this
    /// existed exports the frames it always did.
    #[default]
    CompSetting,
    /// *On for checked layers*: the master goes on in every composition in the
    /// walk, nested ones included; the per-layer switches are left alone,
    /// because they are the checks the phrase names.
    OnForChecked,
    /// *Off for all layers*: the master goes off **and** every layer's own
    /// switch is cleared. Either alone would stop the blur — the master is the
    /// one gate everything passes — but the row says *for all layers*, and a
    /// snapshot in which a layer is still checked would only be true by
    /// accident of which gate was shut.
    OffForAll,
}

/// The export's answer for Retime blend (docs/15 §12A.4, the Time section's
/// second row) — how a fractional source moment becomes pixels
/// ([`lumit_core::retime::Interpolation`], docs/04 §10).
///
/// **Two answers, not the three the motion-blur row has**, and the difference
/// is the model rather than the drawing: Lumit has no composition-wide frame
/// blending master to switch on. A layer's Nearest/Blend/Flow choice *is* its
/// check, so "on for checked layers" and "current settings" would be the same
/// export, and offering both would be a picker where one option does nothing.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum RetimeBlendOverride {
    /// *Current settings*: each layer's (and each Sequence clip's) own policy
    /// stands. The default.
    #[default]
    CompSetting,
    /// *Off for all layers*: every layer and every clip falls back to
    /// [`Interpolation::Nearest`] — the crisp, whole-source-frame export, and
    /// the cheapest one, since neither the blend pair nor the flow field is
    /// asked for.
    OffForAll,
}

/// What the export does to the composition on the way through — the render
/// settings the export drawing puts beside the output format.
#[derive(Clone, Copy, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct RenderOptions {
    /// The resolution/quality tier, the same one the preview uses
    /// (docs/01-GLOSSARY.md §5: Full, Half, Third, Quarter). Full is what an
    /// export wants and what it gets unless something else is asked for.
    pub quality: crate::plan::Quality,
    pub disk_cache: DiskCachePolicy,
    /// Run each layer's effect stack. Off exports the layers unaffected —
    /// the export-time twin of the per-layer fx switch (docs/08 §1.5).
    pub effects: bool,
    /// Honour solo switches. Off exports every visible layer even
    /// when one is soloed for working on — an export of a soloed comp is
    /// almost never what was wanted, but it must be *askable*, not assumed.
    pub honour_solo: bool,
    /// Deliver the guide layers too. Off — the default — is what a
    /// guide layer *is*: reference-only, drawn in the Viewer and absent from
    /// the file, at every depth. On overrides that for the one export that
    /// wants the reference in the picture.
    pub render_guides: bool,
    /// Force motion blur one way for the whole walk, or leave the
    /// compositions' own settings alone ([`MotionBlurOverride`]).
    pub motion_blur: MotionBlurOverride,
    /// Force Retime blend off for the whole walk, or leave each layer's own
    /// policy alone ([`RetimeBlendOverride`]).
    pub retime_blend: RetimeBlendOverride,
    /// Read the proxies instead of the originals. **Off by default,
    /// whatever the project is set to**: a proxy is a working convenience, and
    /// delivery is the one moment it must not apply, so an export takes the
    /// full-resolution files unless it is explicitly asked not to — a draft for
    /// review being the only export a proxy is right for.
    ///
    /// The override lives here rather than being read off the Viewer's state
    /// precisely so the preview-equals-export promise keeps holding in the
    /// direction that matters: what is delivered is decided by the export, and
    /// turning proxies on to work cannot quietly ship the small picture.
    pub use_proxies: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            quality: crate::plan::Quality::default(),
            disk_cache: DiskCachePolicy::default(),
            effects: true,
            honour_solo: true,
            render_guides: false,
            motion_blur: MotionBlurOverride::default(),
            retime_blend: RetimeBlendOverride::default(),
            use_proxies: false,
        }
    }
}

impl RenderOptions {
    /// Whether these options change anything about the document *by
    /// themselves*. Guide layers are not here: skipping them is the default,
    /// so whether it changes this document depends on the document — see
    /// [`apply_render_overrides`].
    pub fn changes_document(&self) -> bool {
        !self.effects
            || !self.honour_solo
            || self.motion_blur != MotionBlurOverride::CompSetting
            || self.retime_blend != RetimeBlendOverride::CompSetting
    }
}

/// Whether any comp in `doc` holds a guide layer.
fn has_guide_layer(doc: &Document) -> bool {
    doc.items.iter().any(|item| {
        matches!(item, ProjectItem::Composition(c) if c.layers.iter().any(|l| l.switches.guide))
    })
}

/// Apply the document-shaped render options to the export's own snapshot —
/// effects off clears every layer's fx switch, solo ignored clears every solo
/// switch, and both apply through nested comps, because "no effects" means no
/// effects anywhere in this export.
///
/// Returns `None` when nothing would change, so the common export keeps the
/// snapshot it was given rather than cloning a whole document to alter
/// nothing. The copy is thrown away when the export finishes and never
/// reaches the project (docs/06 §7.2: baking is invisible).
pub fn apply_render_overrides(doc: &Arc<Document>, opts: &RenderOptions) -> Option<Arc<Document>> {
    // Guide layers leave the delivery the same way: not by a second
    // flag threaded through every walk, but by leaving this snapshot — so the
    // draw builder, the decode planner, the occlusion cull and the frame key
    // all agree, at every depth, that the layer is not there. The Viewer never
    // takes this path, so it keeps drawing them.
    let drop_guides = !opts.render_guides && has_guide_layer(doc);
    // Proxies leave the delivery by the same route, and for the same
    // reason it worked for guide layers: the project's own master switch is one
    // field on the snapshot, so clearing it here makes the decode planner, the
    // frame key and every nested walk agree — at every depth, and without a
    // second flag threaded through any of them — that this export reads the
    // originals. The Viewer never takes this path and keeps its proxies.
    //
    // Guarded on a proxy actually being *switched on* somewhere, not merely on
    // the two flags differing: nearly every project has the master switch on
    // and no proxies at all, and every ordinary export would otherwise clone a
    // whole document to alter a field that changes nothing (`render_guides`
    // takes the same care, for the same reason).
    let set_proxies =
        doc.use_proxies != opts.use_proxies && doc.proxies.values().any(|p| p.enabled);
    if !opts.changes_document() && !drop_guides && !set_proxies {
        return None;
    }
    let mut copy = Document::clone(doc);
    copy.use_proxies = opts.use_proxies;
    for item in &mut copy.items {
        let ProjectItem::Composition(comp) = item else {
            continue;
        };
        // The master switch is a comp setting, so it is set here rather than in
        // the layer loop — and in *every* comp, nested ones included, because
        // "on for checked layers" that stopped at the top comp would leave a
        // precomp's checked layers unblurred inside a blurred export.
        match opts.motion_blur {
            MotionBlurOverride::CompSetting => {}
            MotionBlurOverride::OnForChecked => comp.motion_blur.enabled = true,
            MotionBlurOverride::OffForAll => comp.motion_blur.enabled = false,
        }
        for layer in &mut comp.layers {
            if drop_guides && layer.switches.guide {
                // Reference-only means the whole layer: no picture, no sound,
                // and no solo — guide-ness governs the file, solo governs
                // which layers are looked at, so a soloed guide layer is still
                // absent and the solos it left behind still stand.
                layer.switches.visible = false;
                layer.switches.audible = false;
                layer.switches.solo = false;
                continue;
            }
            if !opts.effects {
                layer.switches.fx = false;
            }
            if !opts.honour_solo {
                layer.switches.solo = false;
            }
            if opts.motion_blur == MotionBlurOverride::OffForAll {
                layer.switches.motion_blur = false;
            }
            if opts.retime_blend == RetimeBlendOverride::OffForAll {
                layer.interpolation = Interpolation::Nearest;
                // A Sequence layer's clips carry their own policy beside the
                // layer's (docs/04 §10), and the decode planner reads the
                // clip's when there is one — so a row left alone here would
                // keep blending inside a sequence.
                if let LayerKind::Sequence { clips } = &mut layer.kind {
                    for clip in clips {
                        clip.interpolation = Interpolation::Nearest;
                    }
                }
            }
        }
    }
    Some(Arc::new(copy))
}

/// What happens the moment an export finishes (docs/07 §11's *When done*).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum WhenDone {
    #[default]
    Nothing,
    /// Play a short sound, so a long export can be left to run.
    MakeANoise,
    /// Show the finished file in the file browser.
    OpenFolder,
}

/// Where the completion sound lives, if it is there at all: beside the
/// executable first (a shipped build's own copy), then the application's data
/// directory (a user's own). `None` when neither exists — the hook is silent
/// rather than faulty when no sound has been supplied.
pub fn done_sound_path() -> Option<PathBuf> {
    let beside_exe = std::env::current_exe().ok().and_then(|exe| {
        exe.parent()
            .map(|dir| dir.join("sounds").join(lumit_project::EXPORT_DONE_SOUND))
    });
    [beside_exe, lumit_project::export_done_sound_path()]
        .into_iter()
        .flatten()
        .find(|p| p.is_file())
}

/// Play the completion sound, if there is one. Answers whether there was a
/// sound to play, and is silent — never an error — when the file is absent,
/// cannot be decoded, or there is no audio device to play it on: a missing
/// ding must never make a finished export look failed.
///
/// Returns at once. Everything happens on a thread of its own, engine
/// included: the audio device's stream handle cannot cross threads, so it is
/// born and dies on the one that keeps it alive for the sound's length.
pub fn play_done_sound() -> bool {
    let Some(path) = done_sound_path() else {
        return false;
    };
    std::thread::spawn(move || {
        let Ok(engine) = lumit_audio::AudioEngine::new() else {
            return;
        };
        let Ok(buffer) = lumit_media::audio::decode_all(&path, engine.device_rate()) else {
            return;
        };
        // A ding, not a track: ten seconds is generous and stops a wrongly
        // supplied file from holding a thread open all afternoon.
        let seconds = buffer.duration_seconds().clamp(0.0, 10.0);
        engine.load(Arc::new(buffer));
        engine.play();
        std::thread::sleep(std::time::Duration::from_secs_f64(seconds + 0.25));
    });
    true
}

/// The pack stage: the finished display frame turned into the exact bytes the
/// encoder is fed (docs/06 §7.4).
///
/// In plain terms: the compositor's answer is one premultiplied,
/// display-encoded RGBA pixel per pixel, at whichever width the export asked
/// the renderer for — eight bits a channel or sixteen. What a file wants may be
/// narrower (no alpha) or differently related (straight alpha). This is where
/// that conversion happens, on the processor, once per frame, and it is pure —
/// which is why it is the one part of colour handling that can be tested
/// without a graphics card.
///
/// The depth is the *input type*, not a setting: `&[u8]` packs an eight-bit
/// file and `&[u16]` a sixteen-bit one, each channel little-endian (the byte
/// order the encoder seam expects). There is nowhere left to widen a signal
/// that was never deep, which is the point of it being typed.
/// [`pack_frame`] for the scene-linear float depths (docs/06 §7.4a): the same
/// channel and alpha decisions, on numbers that are already what the file will
/// hold.
///
/// **No colour transform.** The integer path converts because it is writing a
/// picture into a space a screen will read it in; this is writing the scene,
/// which has no destination space to be converted to. An OpenEXR that had a
/// transfer curve applied would be the one thing the format exists to avoid.
///
/// **No clamp either**, except on alpha, which is coverage and genuinely runs
/// nought to one. The un-multiply divides colour back up and a premultiplied
/// pixel brighter than its own coverage is exactly what an additive blend
/// makes — at eight bits that has to be clipped, here it does not.
#[must_use]
pub fn pack_frame_f32(px: &[f32], channels: Channels, alpha: AlphaMode) -> Vec<u8> {
    let straight = channels == Channels::RgbAlpha && alpha == AlphaMode::Straight;
    let opaque = channels == Channels::Rgb;
    let mut out = Vec::with_capacity(px.len() * 4);
    for chunk in px.chunks_exact(4) {
        let a = chunk[3];
        let mut rgba = [chunk[0], chunk[1], chunk[2], chunk[3]];
        if opaque {
            rgba[3] = 1.0;
        } else if straight && a > 0.0 {
            for c in &mut rgba[..3] {
                *c /= a;
            }
        } else if straight {
            // No coverage: no colour to recover. Zero rather than a division
            // that has no answer.
            rgba[..3].fill(0.0);
        }
        for c in rgba {
            out.extend_from_slice(&c.to_le_bytes());
        }
    }
    out
}

/// [`lumit_core::pixels::letterbox_resize`] for scene-linear floats.
///
/// The integer resize is generic over a `Channel`, which is normalised-integer
/// shaped — it counts to a maximum and rounds — so a float frame cannot ride
/// it. Bilinear, the same filter the `Fast` resample uses, and nothing is
/// clamped on the way through.
#[must_use]
pub fn letterbox_resize_f32(
    px: &[f32],
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    dst_h: u32,
) -> Vec<f32> {
    let (fit_w, fit_h, off_x, off_y) = lumit_core::pixels::fit_contain(src_w, src_h, dst_w, dst_h);
    let (sw, sh) = (src_w.max(1) as usize, src_h.max(1) as usize);
    let mut out = vec![0.0f32; (dst_w as usize) * (dst_h as usize) * 4];
    for y in 0..fit_h as usize {
        // The source row this output row sits on, in continuous coordinates.
        let sy = ((y as f64 + 0.5) * f64::from(src_h) / f64::from(fit_h.max(1)) - 0.5)
            .clamp(0.0, (sh - 1) as f64);
        let (y0, fy) = (sy.floor() as usize, sy - sy.floor());
        let y1 = (y0 + 1).min(sh - 1);
        for x in 0..fit_w as usize {
            let sx = ((x as f64 + 0.5) * f64::from(src_w) / f64::from(fit_w.max(1)) - 0.5)
                .clamp(0.0, (sw - 1) as f64);
            let (x0, fx) = (sx.floor() as usize, sx - sx.floor());
            let x1 = (x0 + 1).min(sw - 1);
            let at = |px_i: usize, py: usize, c: usize| -> f64 {
                f64::from(px.get((py * sw + px_i) * 4 + c).copied().unwrap_or(0.0))
            };
            let to = ((y + off_y as usize) * dst_w as usize + x + off_x as usize) * 4;
            for c in 0..4 {
                let top = at(x0, y0, c) * (1.0 - fx) + at(x1, y0, c) * fx;
                let bot = at(x0, y1, c) * (1.0 - fx) + at(x1, y1, c) * fx;
                if let Some(slot) = out.get_mut(to + c) {
                    *slot = (top * (1.0 - fy) + bot * fy) as f32;
                }
            }
        }
    }
    out
}

pub fn pack_frame<C: lumit_core::pixels::Channel>(
    px: &[C],
    channels: Channels,
    alpha: AlphaMode,
    colour: Option<&ColourTransform>,
) -> Vec<u8> {
    let straight = channels == Channels::RgbAlpha && alpha == AlphaMode::Straight;
    let opaque = channels == Channels::Rgb;
    let mut out = Vec::with_capacity(px.len() * C::BYTES);
    for chunk in px.chunks_exact(4) {
        let a = chunk[3].to_f64();
        let mut rgba = [chunk[0], chunk[1], chunk[2], chunk[3]];
        // The colour transform, where one is asked for. A transfer curve is
        // per-channel and non-linear, so it must see *straight* colour: divide
        // the coverage out, convert, put it back. With no transform this loop
        // is untouched, so an export that names today's space is byte-for-byte
        // the export it always was.
        if let (Some(t), true) = (colour, a > 0.0) {
            let inv = 1.0 / a;
            let done = t.apply([
                rgba[0].to_f64() * inv,
                rgba[1].to_f64() * inv,
                rgba[2].to_f64() * inv,
            ]);
            for (c, v) in rgba[..3].iter_mut().zip(done) {
                *c = C::from_f64(v * a);
            }
        }
        if opaque {
            rgba[3] = C::FULL;
        } else if straight && a > 0.0 && a < C::SCALE {
            // Un-multiply: colour back to full strength wherever there is any
            // coverage. Rounded, and clamped because a premultiplied pixel
            // whose colour exceeds its own coverage (an additive blend can
            // make one) would divide past full scale.
            for c in &mut rgba[..3] {
                *c = C::from_f64(c.to_f64() * C::SCALE / a);
            }
        } else if straight && a == 0.0 {
            // No coverage: no colour to recover. Zero rather than a division
            // that has no answer.
            rgba[..3].fill(C::from_f64(0.0));
        }
        for c in rgba {
            c.write_le(&mut out);
        }
    }
    out
}

/// The crop an export actually applies, from the dialog's two faces: the
/// explicit `T · L · B · R`, or the Viewer's region of interest when *use
/// region of interest* is ticked and a region is set.
///
/// The region wins when it is asked for and exists; otherwise the typed crop
/// stands. A region that is not four finite, increasing fractions is no
/// region, and answers the typed crop rather than nothing.
pub fn crop_for(
    explicit: Crop,
    use_region: bool,
    region: Option<[f64; 4]>,
    w: u32,
    h: u32,
) -> Crop {
    match (use_region, region) {
        (true, Some(r)) => {
            let from_region = Crop::from_region(r, w, h);
            if from_region.is_none() {
                explicit
            } else {
                from_region
            }
        }
        _ => explicit,
    }
}

/// Everything one queued export needs beyond the document snapshot: the
/// format, resolved output size, rates, range, what the picture carries and
/// what happens when it finishes.
///
/// Every field carries `serde`'s default when a stored preset does not name it
/// (`#[serde(default)]`), so a preset saved by an older Lumit still loads and
/// simply takes today's default for whatever it had never heard of.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ExportSpec {
    pub format: ExportFormat,
    /// The delivery frame; None = the composition's own size, exactly as
    /// `fps: None` means the composition's own rate.
    pub target: Option<(u32, u32)>,
    /// The video bitrate: worked out from the size and rate, or the number
    /// that was typed. Meaningless — and unread — for the lossless formats.
    pub bitrate: Bitrate,
    /// Output frame rate; None = the composition's own. A different
    /// rate resamples by nearest comp frame — the honest thing without optical
    /// flow in the export path — and the file is stamped with the chosen rate.
    pub fps: Option<f64>,
    /// The export range in comp frames, end exclusive; None = the work area
    /// when one is set, else the whole comp (the standing behaviour).
    pub range: Option<(usize, usize)>,
    pub include_audio: bool,
    pub audio_bit_rate: i64,
    /// The rate the mix is resampled to and the file is written at, in Hz.
    /// Every source is decoded straight to this rate through the same
    /// resampler the preview mix uses, so no second sampling step exists to
    /// disagree with the first.
    pub audio_rate: u32,
    /// Bits per written sample. Meaningful only where the format has samples
    /// to give a width to ([`FormatCaps::audio_depths`]).
    pub audio_depth: AudioDepth,
    /// One channel or two. Mono folds the comp's stereo mix down; the law is
    /// stated on [`lumit_audio::mix::downmix_to_mono`].
    pub audio_layout: AudioLayout,
    /// Bits per channel in the written file.
    pub depth: BitDepth,
    pub channels: Channels,
    pub alpha: AlphaMode,
    pub colour_space: ColourSpace,
    /// Which filter the resize samples with when the delivered frame is not
    /// the cropped comp's own size. Unread when no resize happens.
    pub resample: lumit_core::pixels::Resample,
    /// Pixels taken off each edge on the way out, already resolved from the
    /// region of interest where that was asked for ([`crop_for`]).
    pub crop: Crop,
    /// What is written into the container about the file.
    pub metadata: lumit_media::encode::Metadata,
    /// How the composition is rendered for this export.
    pub render: RenderOptions,
    pub when_done: WhenDone,
}

impl Default for ExportSpec {
    /// A comp-sized H.264 mp4 with sound — what a plain "Export…" has always
    /// meant — at every setting's own default.
    fn default() -> Self {
        Self {
            format: ExportFormat::Video(lumit_media::encode::VideoCodec::H264),
            target: None,
            bitrate: Bitrate::default(),
            fps: None,
            range: None,
            include_audio: true,
            audio_bit_rate: PRESET_AUDIO_BPS,
            audio_rate: EXPORT_AUDIO_RATE,
            audio_depth: AudioDepth::default(),
            audio_layout: AudioLayout::default(),
            depth: BitDepth::default(),
            channels: Channels::default(),
            alpha: AlphaMode::default(),
            colour_space: ColourSpace::default(),
            resample: lumit_core::pixels::Resample::default(),
            crop: Crop::NONE,
            metadata: lumit_media::encode::Metadata::new(),
            render: RenderOptions::default(),
            when_done: WhenDone::default(),
        }
    }
}

impl ExportSpec {
    /// Refuse a spec the chosen format cannot honour, before a single frame is
    /// rendered. A setting a format cannot carry is a mistake worth naming —
    /// silently ignoring it would deliver a file that is not what was asked
    /// for, and the user would find out from someone else.
    /// [`Self::check`], with the project's loaded colour config to hand.
    ///
    /// This is the delivery half of the asymmetry. A preview whose config has
    /// gone missing degrades calmly to the built-in transform and still shows a
    /// picture; a delivery does not, because a wrong colour space in a file
    /// somebody hands over is worse than an export that did not run. So a name
    /// the config can honour passes here, and every other name refuses —
    /// including the same name a moment after the config moved.
    pub fn check_with_colour(&self, colour: &crate::colour::ColourState) -> Result<(), String> {
        if let Some(name) = self.colour_space.ocio_name() {
            let usable = colour
                .loaded()
                .filter(|l| l.usable())
                .and_then(|l| l.artefact(&crate::colour::Edge::Output(name.to_string())))
                .is_some();
            if !usable {
                return Err(match colour.loaded().and_then(|l| l.problem.clone()) {
                    Some(why) => format!("the colour space \"{name}\" cannot be delivered: {why}"),
                    None => format!(
                        "the colour space \"{name}\" is not in this project's colour config"
                    ),
                });
            }
            // Everything else the plain check asks still applies, minus the
            // build-availability line it would refuse on.
            let mut without = self.clone();
            without.colour_space = ColourSpace::default();
            return without.check();
        }
        self.check()
    }

    pub fn check(&self) -> Result<(), String> {
        let caps = self.format.caps();
        if caps.video && !caps.depths.contains(&self.depth) {
            return Err(format!(
                "{} cannot carry {} colour",
                self.format.extension(),
                self.depth.label()
            ));
        }
        if self.channels == Channels::RgbAlpha && caps.video && !caps.alpha {
            return Err(format!(
                "{} cannot carry an alpha channel",
                self.format.extension()
            ));
        }
        if !self.colour_space.is_available() {
            return Err(format!(
                "the colour space \"{}\" is not available in this build",
                self.colour_space.label()
            ));
        }
        if caps.video && !caps.colour_spaces.contains(&self.colour_space) {
            return Err(format!(
                "{} cannot state that it is {}",
                self.format.extension(),
                self.colour_space.label()
            ));
        }
        if caps.audio && !caps.audio_rates.contains(&self.audio_rate) {
            return Err(format!(
                "{} cannot be written at {} Hz",
                self.format.extension(),
                self.audio_rate
            ));
        }
        if caps.audio && !caps.audio_depths.contains(&self.audio_depth) {
            return Err(format!(
                "{} cannot carry {}-bit sound",
                self.format.extension(),
                self.audio_depth.bits()
            ));
        }
        if !caps.video && !caps.audio {
            return Err("this format can carry neither picture nor sound".to_owned());
        }
        Ok(())
    }

    /// The render settings this export actually runs with.
    ///
    /// **A sound file has no picture, so it takes no picture settings.** Every
    /// field of [`RenderOptions`] is a statement about how the composition is
    /// *drawn* — the tier it draws at, whether the effect stacks run, which
    /// layers the picture looks at, which files it reads. An audio-only export
    /// draws nothing, so it runs at the defaults whatever the spec carries (the
    /// owner's ruling of 2026-08-30; the dialogue dims the whole Composition
    /// group for the same reason).
    ///
    /// It matters because two of them did reach the mix. `honour_solo: false`
    /// clears every layer's solo switch, and the mixer counts solos with
    /// [`lumit_core::model::any_solo`] — every soloed layer, audio-only ones
    /// included — so a picture setting was deciding what a `.wav`
    /// contained. The defaults are the two rules that are *not* picture
    /// settings: solos are honoured, exactly as playback honours them,
    /// and a guide layer stays reference-only at every depth, its sound no more
    /// delivered than its picture.
    pub fn render_options(&self) -> RenderOptions {
        match self.format {
            ExportFormat::Audio(_) => RenderOptions::default(),
            _ => self.render,
        }
    }

    /// The video bitrate this spec runs with, as `(target, peak)` — the typed
    /// numbers, or the worked-out ones for `size` — and `None` for a format
    /// with no bitrate to choose. `size` is the frame actually being written,
    /// which is the composition's own whenever no target was named.
    pub fn resolved_bitrate(&self, size: (u32, u32), fps: f64) -> Option<(i64, Option<i64>)> {
        if !self.format.caps().bit_rate {
            return None;
        }
        let codec = match self.format {
            ExportFormat::Video(c) => c,
            // Audio-only: the AAC bitrate is its own field; there is no video
            // rate to work out.
            _ => return None,
        };
        Some(match self.bitrate {
            // No bitrate at all: the encoder chooses its own quality.
            Bitrate::EncoderDefault => return None,
            Bitrate::Auto => {
                let (w, h) = self.target.unwrap_or(size);
                let (t, p) = auto_bitrate(w, h, fps, codec);
                (t, Some(p))
            }
            Bitrate::Manual {
                target_bps,
                peak_bps,
            } => (
                target_bps,
                peak_bps.or_else(|| Some((target_bps as f64 * PEAK_MULTIPLE).round() as i64)),
            ),
        })
    }
}

/// A chosen output rate as the exact rational the encoder is stamped with:
/// thousandths, reduced — 29.97 → 2997/100, 60 → 60/1. Millihertz is finer
/// than any delivery rate needs and keeps the arithmetic in integers.
pub fn fps_rational(fps: f64) -> (i32, i32) {
    let clamped = fps.clamp(1.0, 1000.0);
    let mut num = (clamped * 1000.0).round() as i64;
    let mut den = 1000i64;
    let gcd = {
        let (mut a, mut b) = (num, den);
        while b != 0 {
            (a, b) = (b, a % b);
        }
        a.max(1)
    };
    num /= gcd;
    den /= gcd;
    (num as i32, den as i32)
}

/// One export waiting its turn. The document and audio jobs are snapshotted
/// at queue time (docs/06 §7.1): later edits never alter a queued item.
pub struct QueuedExport {
    pub doc: Arc<Document>,
    pub comp_id: Uuid,
    pub items: HashMap<Uuid, ItemInfo>,
    pub audio: Vec<AudioJob>,
    pub out_path: PathBuf,
    pub spec: ExportSpec,
}

pub fn start(
    doc: Arc<Document>,
    comp_id: Uuid,
    audio: Vec<AudioJob>,
    out_path: PathBuf,
    spec: ExportSpec,
) -> ExportHandle {
    let (tx, events) = channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    std::thread::spawn(move || {
        let result = run(&doc, comp_id, &audio, &out_path, &spec, &tx, &flag);
        let _ = match result {
            Ok(()) if flag.load(Ordering::Relaxed) => {
                let _ = std::fs::remove_file(&out_path); // no half files
                tx.send(ExportEvent::Failed("cancelled".into()))
            }
            Ok(()) => tx.send(ExportEvent::Done(out_path)),
            Err(e) => {
                let _ = std::fs::remove_file(&out_path);
                tx.send(ExportEvent::Failed(e))
            }
        };
    });
    ExportHandle { events, cancel }
}

/// Decode every audio job (resampled to `rate`), lay each on the comp strip
/// at its offset and trim, and sum — the one mixdown all comp audio flows
/// through: preview playback, beat detection, and export, so they cannot
/// disagree about what the comp sounds like.
pub fn mixdown(jobs: &[AudioJob], rate: u32, duration_s: f64) -> Vec<f32> {
    mixdown_at(jobs, rate, duration_s, 1.0)
}

/// As [`mixdown`], through a composition's **master fader** (linear gain) —
/// what the export writes and what playback hears. `mixdown` itself
/// stays at unity for the reader that wants the mix *before* the desk: beat
/// detection, whose onsets are relative, and which must not lose its beats
/// because somebody pulled the master down.
pub fn mixdown_at(jobs: &[AudioJob], rate: u32, duration_s: f64, master_gain: f32) -> Vec<f32> {
    mixdown_counting(jobs, rate, duration_s, master_gain, &mut |_| {})
}

/// As [`mixdown_at`], counting the jobs off as each one is summed.
///
/// Decoding is where a mixdown spends its seconds. Each job reads only the
/// stretch of its file it places, through the pool the mixer plays from
/// ([`lumit_audio::stream`]), is added to the sum and let go: a cut of two
/// thousand clips holds the sum and one clip at a time, where it used to hold
/// two thousand whole files. `on_decoded` is handed how many of `jobs` are
/// done, and it is called for a source that would not decode as well as for
/// one that did.
pub fn mixdown_counting(
    jobs: &[AudioJob],
    rate: u32,
    duration_s: f64,
    master_gain: f32,
    on_decoded: &mut dyn FnMut(usize),
) -> Vec<f32> {
    let pool = lumit_audio::stream::pool();
    let sourced: Vec<(Arc<Source>, &AudioJob)> = jobs
        .iter()
        .map(|job| (pool.source(&job.path, rate), job))
        .collect();
    mix_sources(&sourced, rate, duration_s, master_gain, on_decoded)
}

/// As [`mixdown`], but over buffers already in memory.
pub fn mixdown_prepared(
    decoded: &[(std::sync::Arc<lumit_media::AudioBuffer>, AudioJob)],
    rate: u32,
    duration_s: f64,
    master_gain: f32,
) -> Vec<f32> {
    let sourced: Vec<(Arc<Source>, &AudioJob)> = decoded
        .iter()
        .map(|(b, j)| (Source::whole(Arc::clone(b)), j))
        .collect();
    mix_sources(&sourced, rate, duration_s, master_gain, &mut |_| {})
}

/// Where a job lands: [`lumit_audio::mix::place_on_timeline`] over its
/// source. `wait` finds a file's exact length first, which an export always
/// does and the live plan does for a job whose samples it is about to read;
/// without it a file whose length nobody has found yet is placed as if it
/// ran on, and reads as silence past its end, which mixes to the same sound.
fn place(source: &Source, job: &AudioJob, rate: u32, wait: bool) -> Option<(i64, usize, usize)> {
    let frames = if wait {
        source.frames_exactly()
    } else {
        // Long enough that no clip is cut short by it, short enough that
        // adding to it cannot overflow.
        source.frames().unwrap_or(usize::MAX / 4)
    };
    lumit_audio::mix::place_on_timeline(job.in_s, job.out_s, job.offset_s, frames, rate)
}

/// **The buses of a mix**, each with the index of the job it stands in line
/// at: every job arriving through a Precomp layer that carries a rack, read,
/// run through its own racks, summed and run through the layer's
/// ([`bus_runs`]). These are the jobs that have to be in memory together,
/// because a rack hears a sum.
fn buses(sourced: &[(Arc<Source>, &AudioJob)], rate: u32, offline: bool) -> Vec<(usize, MixRun)> {
    /// One member of some bus, read and its own racks run.
    struct Member<'a> {
        index: usize,
        job: &'a AudioJob,
        start_frame: i64,
        samples: Vec<f32>,
    }
    let members: Vec<Member<'_>> = sourced
        .iter()
        .enumerate()
        .filter(|(_, (_, job))| in_a_bus(job))
        .filter_map(|(index, (source, job))| {
            let (start_frame, src_start, len) = place(source, job, rate, true)?;
            let dry = source.read(src_start, len, offline);
            Some(match job_bake(job, &dry, start_frame, rate, offline) {
                Some((wet, latency)) => Member {
                    index,
                    job,
                    start_frame: start_frame - i64::from(latency),
                    samples: wet,
                },
                None => Member {
                    index,
                    job,
                    start_frame,
                    samples: dry,
                },
            })
        })
        .collect();
    if members.is_empty() {
        return Vec::new();
    }
    let staged: Vec<PlacedJob<'_>> = members
        .iter()
        .map(|m| PlacedJob {
            job: m.job,
            start_frame: m.start_frame,
            samples: &m.samples,
        })
        .collect();
    bus_runs(&staged, rate, offline)
        .into_iter()
        .map(|run| (members[run.first].index, run))
        .collect()
}

/// The shared placement and sum over sources (each at `rate`), the way an
/// export wants it: every sample the one a decode of the whole file holds,
/// every rack run offline, and nothing held longer than it takes to add it.
///
/// The runs are added in the order of the jobs, a bus where its first job
/// stands, which is the order the live plan plays them in: the two sum the
/// same numbers in the same order.
fn mix_sources(
    sourced: &[(Arc<Source>, &AudioJob)],
    rate: u32,
    duration_s: f64,
    master_gain: f32,
    on_done: &mut dyn FnMut(usize),
) -> Vec<f32> {
    let total_frames = (duration_s * f64::from(rate)).round().max(0.0) as usize;
    let mut out = vec![0.0f32; total_frames * 2];
    // The bus stage first: a Precomp layer carrying a rack sums what arrives
    // through it and the rack hears the sum, which is the one thing no amount
    // of per-job arithmetic can do.
    let mut buses = buses(sourced, rate, true).into_iter().peekable();
    for (index, (source, job)) in sourced.iter().enumerate() {
        if in_a_bus(job) {
            if let Some((_, run)) = buses.next_if(|(at, _)| *at == index) {
                if let RunOf::Bus(samples) = &run.of {
                    lumit_audio::mix::add_placed(
                        &mut out,
                        &lumit_audio::mix::PlacedAudio {
                            start_frame: run.start_frame,
                            samples,
                            gain: run.gain,
                            envelope: run.envelope,
                        },
                    );
                }
            }
        } else if let Some((start_frame, src_start, len)) = place(source, job, rate, true) {
            let dry = source.read(src_start, len, true);
            // The export runs the chains **offline** (docs/impl/audio-plugins.md
            // §3): no deadline, and the plugin may take its slower path.
            //
            // Latency compensation: the processed run is the placed span plus
            // the chain's own delay, put down that many frames earlier so the
            // wet sound lands where the dry did. Its length is the run's own,
            // not the span's, which is what lets a tail ring on past the out
            // point.
            let (samples, start_frame) = match job_bake(job, &dry, start_frame, rate, true) {
                Some((wet, latency)) => (wet, start_frame - i64::from(latency)),
                None => (dry, start_frame),
            };
            let (gain, envelope) = volume_bake(job, start_frame, samples.len() / 2, rate);
            lumit_audio::mix::add_placed(
                &mut out,
                &lumit_audio::mix::PlacedAudio {
                    start_frame,
                    samples: &samples,
                    gain,
                    envelope,
                },
            );
        }
        on_done(index + 1);
    }
    for s in &mut out {
        *s = (*s * master_gain).clamp(
            -lumit_audio::mix::MASTER_CEILING,
            lumit_audio::mix::MASTER_CEILING,
        );
    }
    out
}

/// What the racks made of each job the last time a plan was built, so the next
/// build runs only the racks an edit changed.
///
/// # In plain terms
///
/// A rack on a clip is run over the clip's whole span when the plan is built,
/// and the plan is built again after every edit. Without this a cut with a
/// rack on two hundred clips ran two hundred racks for every trim anywhere.
/// A bake depends only on the stretch of file it was given, where that sits
/// on the timeline, and the racks themselves, so a job that matches the last
/// build on all three is handed the same sound back.
///
/// Holds what the last plan held and no more: a build takes what it can use
/// and leaves behind only what it used.
#[derive(Default)]
pub struct RackBakes {
    held: Vec<(BakeKey, Baked)>,
    /// Whatever else the racks' answers depend on that the keys cannot see:
    /// the caller's own stamp, and a changed one empties the lot.
    stamp: u64,
}

/// What a job's racks came to: the processed sound and the latency it is
/// placed early by, or `None` where neither rack opened anything as audio.
type Baked = Option<(Arc<Source>, u32)>;

#[derive(PartialEq)]
struct BakeKey {
    item: Uuid,
    rate: u32,
    start_frame: i64,
    src_start: usize,
    len: usize,
    clip_chain: Option<Arc<AudioChain>>,
    chain: Option<Arc<AudioChain>>,
}

impl RackBakes {
    /// Forget every bake unless `stamp` is the one they were made under. The
    /// caller folds in what a rack's sound depends on beyond the job itself,
    /// which today is the list of plugins switched off for now.
    pub fn stamped(&mut self, stamp: u64) {
        if self.stamp != stamp {
            self.held.clear();
            self.stamp = stamp;
        }
    }

    /// How many bytes of processed sound the bakes hold.
    #[must_use]
    pub fn bytes(&self) -> usize {
        self.held
            .iter()
            .filter_map(|(_, baked)| baked.as_ref())
            .map(|(source, _)| source.frames().unwrap_or(0) * 2 * std::mem::size_of::<f32>())
            .sum()
    }
}

/// Place the jobs on the comp strip as a live [`MixPlan`], and say which
/// mixer strip each of its meter slots belongs to.
///
/// The same placement, racks, bus stage and Volume bake the export mixes with
/// ([`mixdown_counting`]), in the same order, so playback sounds like the
/// file. Two things are said differently. The racks run in realtime and not
/// offline. And a job with no rack on it reads nothing here: it names its
/// source and the stretch it plays, and the blocks are decoded as the
/// playhead nears them. A job whose source `source_of` does not know
/// contributes nothing.
///
/// `bakes` carries each racked job's processed sound from one build to the
/// next ([`RackBakes`]).
pub fn live_plan(
    jobs: &[AudioJob],
    source_of: &dyn Fn(&AudioJob) -> Option<Arc<Source>>,
    rate: u32,
    duration_s: f64,
    master_db: f64,
    bakes: &mut RackBakes,
) -> (Arc<MixPlan>, Vec<Uuid>) {
    let total_frames = (duration_s * f64::from(rate)).round().max(0.0) as usize;
    let sourced: Vec<(Arc<Source>, &AudioJob)> = jobs
        .iter()
        .filter_map(|job| Some((source_of(job).filter(|s| s.rate() == rate)?, job)))
        .collect();
    // Meter slots in first-sounding order, one per strip: several jobs from
    // one Precomp layer share its slot, and past the bank's size the extras
    // play unmetered rather than being dropped.
    let mut strips: Vec<Uuid> = Vec::new();
    let mut slot_of = |layer: Uuid| {
        let slot = match strips.iter().position(|s| *s == layer) {
            Some(at) => at,
            None => {
                strips.push(layer);
                strips.len() - 1
            }
        };
        u8::try_from(slot)
            .ok()
            .filter(|s| usize::from(*s) < lumit_audio::meter::MAX_STRIPS)
            .unwrap_or(lumit_audio::mix::NO_METER)
    };
    let mut kept: Vec<(BakeKey, Baked)> = Vec::new();
    let mut clips = Vec::with_capacity(sourced.len());
    let mut buses = buses(&sourced, rate, false).into_iter().peekable();
    for (index, (source, job)) in sourced.iter().enumerate() {
        if in_a_bus(job) {
            // The bus stage: everything arriving through a Precomp layer that
            // carries a rack was summed and run through it, and comes back as
            // one run of its own on that layer's strip (docs/09 §3.1).
            if let Some((_, run)) = buses.next_if(|(at, _)| *at == index) {
                if let RunOf::Bus(samples) = run.of {
                    let frames = samples.len() / 2;
                    clips.push(lumit_audio::mix::PlacedClip {
                        source: Source::whole(Arc::new(lumit_media::AudioBuffer { rate, samples })),
                        start_frame: run.start_frame,
                        src_start: 0,
                        len: frames,
                        gain: run.gain,
                        envelope: run.envelope.map(Arc::new),
                        meter: slot_of(run.layer),
                    });
                }
            }
            continue;
        }
        let racked = job.chain.is_some() || job.clip_chain.is_some();
        let Some((start_frame, src_start, len)) = place(source, job, rate, racked) else {
            continue;
        };
        // The clip's insert chain and then the layer's, ahead of Volume and
        // Pan. The processed span **replaces** the file in the plan, so the
        // realtime callback plays finished sound and never waits on a
        // plugin's process; a job whose stacks open nothing keeps its file
        // and reads it in blocks like any other.
        let baked: Baked = if racked {
            let key = BakeKey {
                item: job.item,
                rate,
                start_frame,
                src_start,
                len,
                clip_chain: job.clip_chain.clone(),
                chain: job.chain.clone(),
            };
            let baked = match bakes.held.iter().position(|(k, _)| *k == key) {
                Some(at) => bakes.held.swap_remove(at).1,
                None => {
                    let dry = source.read(src_start, len, false);
                    job_bake(job, &dry, start_frame, rate, false).map(|(samples, latency)| {
                        (
                            Source::whole(Arc::new(lumit_media::AudioBuffer { rate, samples })),
                            latency,
                        )
                    })
                }
            };
            kept.push((key, baked.clone()));
            baked
        } else {
            None
        };
        let (source, start_frame, src_start, len) = match baked {
            // Placed the chain's summed latency earlier, so the wet lands
            // where the dry did, and as long as the run came back, so a tail
            // rings on past the out point.
            Some((wet, latency)) => {
                let frames = wet.frames().unwrap_or(0);
                (wet, start_frame - i64::from(latency), 0, frames)
            }
            None => (Arc::clone(source), start_frame, src_start, len),
        };
        let (gain, envelope) = volume_bake(job, start_frame, len, rate);
        clips.push(lumit_audio::mix::PlacedClip {
            source,
            start_frame,
            src_start,
            len,
            gain,
            envelope: envelope.map(Arc::new),
            meter: slot_of(job.layer),
        });
    }
    bakes.held = kept;
    strips.truncate(lumit_audio::meter::MAX_STRIPS);
    (
        Arc::new(MixPlan {
            clips,
            total_frames,
            master_gain: lumit_audio::mix::db_to_gain(master_db),
        }),
        strips,
    )
}

/// How many audio samples (per channel) belong before the end of video
/// frame `frame_count` — the A/V interleaving rule. Cumulative rounding, so
/// the running total never drifts from `frames / fps × rate`.
pub fn audio_samples_through(frame_count: usize, fps: f64, rate: u32) -> usize {
    if fps <= 0.0 {
        return 0;
    }
    ((frame_count as f64 / fps) * f64::from(rate)).round() as usize
}

fn run(
    doc: &Arc<Document>,
    comp_id: Uuid,
    audio_jobs: &[AudioJob],
    out_path: &std::path::Path,
    spec: &ExportSpec,
    tx: &Sender<ExportEvent>,
    cancel: &AtomicBool,
) -> Result<(), String> {
    // The render settings that change the document do it on this export's own
    // throwaway snapshot, never on the project (docs/06 §7.2).
    let overridden = apply_render_overrides(doc, &spec.render_options());
    let doc = overridden.as_ref().unwrap_or(doc);
    let comp = doc.comp(comp_id).ok_or("composition missing")?;
    let fps = comp.frame_rate.fps().max(1.0);
    let comp_frames = (comp.duration.0.to_f64() * fps).round().max(1.0) as usize;
    // The range: the dialogue's own when it set one, else the work area, else
    // the whole comp (docs/01-GLOSSARY.md).
    let (first, end) = match spec.range {
        Some((a, b)) => {
            let s = a.min(comp_frames.saturating_sub(1));
            let e = b.clamp(s + 1, comp_frames);
            (s, e)
        }
        None => match comp.work_area {
            Some((a, b)) => {
                let s = ((a.0.to_f64() * fps).round() as usize).min(comp_frames.saturating_sub(1));
                let e = ((b.0.to_f64() * fps).round() as usize).clamp(s + 1, comp_frames);
                (s, e)
            }
            None => (0, comp_frames),
        },
    };
    // The output rate. A rate other than the comp's resamples by nearest comp
    // frame over the same wall-clock span, so a 60 fps comp exported at 30
    // shows every other frame and lasts exactly as long.
    let out_fps = spec.fps.unwrap_or(fps).clamp(1.0, 1000.0);
    let span_seconds = (end - first) as f64 / fps;
    let total = ((span_seconds * out_fps).round() as usize).max(1);
    let _ = tx.send(ExportEvent::Progress { frame: 0, total });

    // The comp's audio, mixed exactly as playback mixes it, then cut to the
    // export range and padded so sound and picture end together.
    let rate = spec.audio_rate;
    let chans = usize::from(spec.audio_layout.channels());
    // Sound only joins a container that can hold it: a folder of stills has
    // nowhere to put it, and the dialogue says so rather than silently
    // dropping it. An audio-only export is nothing *but* sound, so the
    // include-audio tick has no say there.
    let caps = spec.format.caps();
    let wants_audio = caps.audio && (spec.include_audio || !caps.video);
    // A silent comp exported as sound still writes silence of the right
    // length — an empty .wav would look like a failure that wasn't one — but
    // a video export of a silent comp carries no audio stream at all rather
    // than a mute one.
    let audio_mix: Option<Vec<f32>> = if wants_audio && (!audio_jobs.is_empty() || !caps.video) {
        let full = mixdown_at(
            audio_jobs,
            rate,
            comp.duration.0.to_f64(),
            lumit_audio::mix::db_to_gain(comp.master_volume_db),
        );
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        // The cut starts where the range starts on the comp's own clock, and
        // covers the output's duration — total frames at the *output* rate —
        // so sound and picture end together whatever rate was chosen.
        let start = audio_samples_through(first, fps, rate).min(full.len() / 2);
        let expect = audio_samples_through(total, out_fps, rate);
        let mut cut = full[start * 2..(start + expect).min(full.len() / 2) * 2].to_vec();
        cut.resize(expect * 2, 0.0);
        // The mix is stereo throughout — playback's own mix, so preview and
        // export cannot disagree — and folds down once, at the very end,
        // where nothing else reads it.
        Some(match spec.audio_layout {
            AudioLayout::Stereo => cut,
            AudioLayout::Mono => lumit_audio::mix::downmix_to_mono(&cut),
        })
    } else {
        None
    };

    // Sound with no picture needs no compositor and no graphics card at all:
    // the mix is already made, and there is nothing to render.
    if let ExportFormat::Audio(format) = spec.format {
        return run_audio_only(
            out_path,
            format,
            spec,
            audio_mix.as_deref(),
            total,
            tx,
            cancel,
        );
    }

    // The export renders through the SAME walk the Viewer does — the headless
    // preview at full decode quality (preview == export by
    // construction, gated by the bit-identity matrix in `headless::tests`).
    // Its own renderer on its own device, so an export never contends with the
    // Viewer's GPU work.
    let mut renderer =
        crate::headless::HeadlessRenderer::new().map_err(|e| format!("export renderer: {e}"))?;
    // A layer whose model engine will not paint stops the run here rather than
    // being drawn with the built-in engine and written to the file under the
    // model's name. The pre-flight turns away everything the document alone can
    // show; this is the rest of it (docs/08 §3.1, docs/impl/addons.md §6.3).
    renderer.refuse_substitution();
    // The project's colour config, before anything is written: the refusal has
    // to happen with the config in hand, and it has to happen before a file
    // exists rather than halfway through one.
    renderer.sync_colour(doc);
    spec.check_with_colour(renderer.colour())?;
    // A config's space is delivered by binding its baked table to the SAME
    // display blit the Viewer presents through (docs/impl/ocio.md §5.2). Not a
    // second transform at the pack stage: that would be a second implementation
    // of one transform in the delivery path, which is the exact structure the
    // preview-equals-export promise forbids.
    renderer.set_colour_output(spec.colour_space.ocio_name().map(str::to_owned));
    let (out_num, out_den) = fps_rational(out_fps);
    // One sink, two shapes: the mp4 muxer, or one image file per frame. The
    // loop below is shared — a second frame loop would be a second chance to
    // disagree about sampling, cancellation or progress.
    // The crop happens in composition pixels, so the picture that
    // leaves the compositor is cropped first and sized afterwards. When the
    // delivery size *is* the comp's own — every Custom export — the cropped
    // size becomes the file's size, which is what cropping is for; a preset
    // that asked for a different frame letterboxes the cropped picture into
    // it, exactly as an uncropped one does.
    let (crop_w, crop_h) = spec.crop.output_size(comp.width, comp.height);
    let delivered = match spec.target {
        Some(t) if t != (comp.width, comp.height) => t,
        _ => (crop_w, crop_h),
    };
    let mut sink = match spec.format {
        ExportFormat::Video(codec) => {
            // Encoded frame dimensions must be even for 4:2:0 H.264/HEVC.
            let (tw, th) = (delivered.0 & !1, delivered.1 & !1);
            let (tw, th) = (tw.max(2), th.max(2));
            let audio_settings = audio_mix
                .as_ref()
                .map(|_| lumit_media::encode::AudioSettings {
                    rate,
                    bit_rate: spec.audio_bit_rate,
                    codec: lumit_media::encode::AudioCodec::Aac,
                    channels: spec.audio_layout.channels(),
                });
            let (bit_rate, max_rate) = match spec.resolved_bitrate((tw, th), out_fps) {
                Some((target, peak)) => (Some(target), peak),
                None => (None, None),
            };
            let encoder = lumit_media::Encoder::open(
                out_path,
                Some(&lumit_media::encode::VideoSettings {
                    codec,
                    width: tw,
                    height: th,
                    fps_num: out_num,
                    fps_den: out_den,
                    bit_rate,
                    max_rate,
                    colour: spec.colour_space.tags(),
                }),
                audio_settings.as_ref(),
                &spec.metadata,
            )
            .map_err(|e| e.to_string())?;
            let _ = tx.send(ExportEvent::Encoder(encoder.encoder_label()));
            Sink::Video {
                encoder,
                size: (tw, th),
            }
        }
        ExportFormat::Images(format) => {
            // Stills have no chroma subsampling, so no evenness rule.
            let (tw, th) = (delivered.0.max(1), delivered.1.max(1));
            let encoder = lumit_media::encode::ImageSequenceEncoder::open(
                out_path,
                format,
                tw,
                th,
                out_num,
                out_den,
                spec.depth,
                spec.colour_space.tags(),
            )
            .map_err(|e| e.to_string())?;
            let _ = tx.send(ExportEvent::Encoder(format.label()));
            Sink::Images {
                encoder,
                size: (tw, th),
                written: 0,
            }
        }
        // Handled above: sound with no picture never reaches the frame loop.
        ExportFormat::Audio(_) => return Err("audio-only export took the picture path".into()),
    };
    let resize = sink.size() != (crop_w, crop_h);
    // Worked out once: the primaries matrix and the destination curve are the
    // same for every frame, and deriving them per pixel would be the same
    // answer two million times over.
    let colour = spec.colour_space.transform();

    let mut audio_fed = 0usize;
    for frame_n in 0..total {
        if cancel.load(Ordering::Relaxed) {
            sink.remove_written(out_path);
            return Ok(());
        }
        // The comp frame under this output frame: exact when the rates match
        // (the rounding is then of an integer), nearest otherwise.
        let src = first + ((frame_n as f64) * fps / out_fps).round() as usize;
        let src = src.min(end.saturating_sub(1));
        // Crop in composition pixels first, then letterbox into the delivery
        // frame when the size was changed, then pack to what the file carries.
        // The two arms differ only in how wide a channel is: a sixteen-bit
        // export reads the composite back at sixteen bits and stays there, so
        // the extra width is the pipeline's own rather than a stretched byte.
        let (tw, th) = sink.size();
        let rgba = match spec.depth {
            BitDepth::Eight => {
                let (px, _, _) =
                    renderer.render_preview(doc, comp_id, src as u64, spec.render.quality, 1.0)?;
                let px = spec.crop.apply(&px, comp.width, comp.height, 4);
                let px = if resize {
                    lumit_core::pixels::letterbox_resize(&px, crop_w, crop_h, tw, th, spec.resample)
                } else {
                    px
                };
                pack_frame(&px, spec.channels, spec.alpha, colour.as_ref())
            }
            BitDepth::Sixteen => {
                let (px, _, _) =
                    renderer.render_preview16(doc, comp_id, src as u64, spec.render.quality)?;
                let px = spec.crop.apply(&px, comp.width, comp.height, 4);
                let px = if resize {
                    lumit_core::pixels::letterbox_resize(&px, crop_w, crop_h, tw, th, spec.resample)
                } else {
                    px
                };
                pack_frame(&px, spec.channels, spec.alpha, colour.as_ref())
            }
            // The float depths read the composite back as it stands: no
            // display transform, no curve, no clamp (docs/06 §7.4a). The
            // channel and alpha choices still apply — they are about what the
            // file carries, not about how bright it is — and the resize runs
            // on the floats so a delivered EXR is filtered rather than
            // point-sampled.
            BitDepth::Half | BitDepth::Float => {
                let (px, _, _) = renderer.render_preview_linear(
                    doc,
                    comp_id,
                    src as u64,
                    spec.render.quality,
                )?;
                let px = spec.crop.apply(&px, comp.width, comp.height, 4);
                let px = if resize {
                    letterbox_resize_f32(&px, crop_w, crop_h, tw, th)
                } else {
                    px
                };
                pack_frame_f32(&px, spec.channels, spec.alpha)
            }
        };
        if let Err(e) = sink.write_rgba(&rgba) {
            // A folder of stills that failed half-way is tidied rather than
            // left as a trap that looks like a finished export.
            sink.remove_written(out_path);
            return Err(e);
        }
        // Interleave: after each picture frame, the samples that cover it,
        // so the muxer keeps sound and picture together in the file.
        if let (Some(mix), Sink::Video { encoder, .. }) = (&audio_mix, &mut sink) {
            let upto = audio_samples_through(frame_n + 1, out_fps, rate).min(mix.len() / chans);
            if upto > audio_fed {
                encoder
                    .write_audio(&mix[audio_fed * chans..upto * chans])
                    .map_err(|e| e.to_string())?;
                audio_fed = upto;
            }
        }
        let _ = tx.send(ExportEvent::Progress {
            frame: frame_n + 1,
            total,
        });
    }
    // Any samples the per-frame rounding left behind.
    if let (Some(mix), Sink::Video { encoder, .. }) = (&audio_mix, &mut sink) {
        if mix.len() / chans > audio_fed {
            encoder
                .write_audio(&mix[audio_fed * chans..])
                .map_err(|e| e.to_string())?;
        }
    }
    sink.finish()
}

/// Write the comp's mix with no picture at all (docs/06 §7.4). No compositor,
/// no graphics card: the mixdown above is the whole export, so this feeds it
/// to the muxer in one-second helpings and reports progress against the same
/// frame count a video export would have written.
fn run_audio_only(
    out_path: &std::path::Path,
    format: AudioFormat,
    spec: &ExportSpec,
    mix: Option<&[f32]>,
    total: usize,
    tx: &Sender<ExportEvent>,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let rate = spec.audio_rate;
    let chans = usize::from(spec.audio_layout.channels());
    let mut encoder = lumit_media::Encoder::open(
        out_path,
        None,
        Some(&lumit_media::encode::AudioSettings {
            rate,
            bit_rate: spec.audio_bit_rate,
            codec: format.codec(spec.audio_depth),
            channels: spec.audio_layout.channels(),
        }),
        &spec.metadata,
    )
    .map_err(|e| e.to_string())?;
    let _ = tx.send(ExportEvent::Encoder(encoder.encoder_label()));

    // A comp with no audible layer still exports a file — of silence, of the
    // right length. An empty .wav would look like a failure that wasn't one.
    let silence;
    let mix = match mix {
        Some(m) => m,
        None => {
            silence = Vec::new();
            &silence
        }
    };
    let chunk = rate as usize * chans;
    for (n, block) in mix.chunks(chunk).enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        encoder.write_audio(block).map_err(|e| e.to_string())?;
        // Progress against the picture's own clock, so the queue row reads
        // the same way whatever an item writes.
        let done = ((n + 1) * chunk / chans).min(mix.len() / chans);
        let frame = if mix.is_empty() {
            total
        } else {
            (done * total / (mix.len() / chans).max(1)).min(total)
        };
        let _ = tx.send(ExportEvent::Progress { frame, total });
    }
    encoder.finish().map_err(|e| e.to_string())?;
    let _ = tx.send(ExportEvent::Progress {
        frame: total,
        total,
    });
    Ok(())
}

/// Where the rendered frames go: the mp4 muxer, or the numbered stills. One
/// type so the frame loop stays single.
enum Sink {
    Video {
        encoder: lumit_media::Encoder,
        size: (u32, u32),
    },
    Images {
        encoder: lumit_media::encode::ImageSequenceEncoder,
        size: (u32, u32),
        /// Frames written so far — exactly the files a cancel removes.
        written: usize,
    },
}

impl Sink {
    fn size(&self) -> (u32, u32) {
        match self {
            Sink::Video { size, .. } | Sink::Images { size, .. } => *size,
        }
    }

    fn write_rgba(&mut self, rgba: &[u8]) -> Result<(), String> {
        match self {
            Sink::Video { encoder, .. } => encoder.write_rgba(rgba).map_err(|e| e.to_string()),
            Sink::Images {
                encoder, written, ..
            } => {
                encoder.write_rgba(rgba).map_err(|e| e.to_string())?;
                *written += 1;
                Ok(())
            }
        }
    }

    fn finish(&mut self) -> Result<(), String> {
        match self {
            Sink::Video { encoder, .. } => encoder.finish().map_err(|e| e.to_string()),
            Sink::Images { encoder, .. } => encoder.finish().map_err(|e| e.to_string()),
        }
    }

    /// Remove what a cancelled or failed image export left behind. The mp4
    /// path needs nothing here — its half file is removed by the caller, which
    /// cannot know a sequence's file names; this does.
    fn remove_written(&self, chosen_path: &std::path::Path) {
        if let Sink::Images { written, .. } = self {
            for n in 1..=*written {
                let _ =
                    std::fs::remove_file(lumit_media::encode::sequence_frame_path(chosen_path, n));
            }
        }
    }
}

/// Coverage bytes → white RGBA whose alpha is the coverage (the layer-mask
/// texture format the compositor samples).
pub fn mask_rgba(coverage: &[u8]) -> Vec<u8> {
    coverage.iter().flat_map(|c| [255, 255, 255, *c]).collect()
}

/// CameraPose (core model) -> GPU camera matrix: the single conversion both
/// the preview and the export path share, so they cannot disagree.
pub fn camera_mat(
    comp_w: u32,
    comp_h: u32,
    pose: lumit_core::model::CameraPose,
) -> lumit_gpu::Mat4 {
    lumit_gpu::camera_matrix(
        comp_w as f32,
        comp_h as f32,
        pose.zoom as f32,
        (
            pose.position.0 as f32,
            pose.position.1 as f32,
            pose.position.2 as f32,
        ),
        (
            pose.rotation_deg.0 as f32,
            pose.rotation_deg.1 as f32,
            pose.rotation_deg.2 as f32,
        ),
    )
}

/// Collect the ItemInfo map from probed media (cheap — it only reads the
/// frontend's probe cache, never touches disk). `slate_size` is the exported
/// comp's dimensions, used to size the missing-footage slate exactly as the
/// preview does.
pub fn item_infos(
    doc: &Document,
    probes: &dyn crate::source::SourceProbes,
    slate_size: (u32, u32),
) -> HashMap<Uuid, ItemInfo> {
    let mut map = HashMap::new();
    for item in &doc.items {
        let ProjectItem::Footage(f) = item else {
            continue;
        };
        let probe = probes.probe(f.id);
        if let Some((fps, _w, _h, frames)) = probe.video() {
            map.insert(
                f.id,
                ItemInfo {
                    source: lumit_media::MediaSource {
                        path: PathBuf::from(&f.media.absolute_path),
                        sequence_fps: f.sequence_fps(),
                        source_layer: f.source_layer,
                    },
                    fps,
                    frames,
                    missing: None,
                },
            );
        } else if probe.slates() {
            // Missing/unreadable media is carried, not skipped, so export
            // renders the same slate the Viewer shows. Audio-only and
            // unprobed items are simply absent: no picture, and — crucially —
            // no slate over a perfectly healthy sound file.
            map.insert(
                f.id,
                ItemInfo {
                    source: lumit_media::MediaSource {
                        path: PathBuf::from(&f.media.absolute_path),
                        sequence_fps: f.sequence_fps(),
                        source_layer: f.source_layer,
                    },
                    fps: 1.0,
                    frames: 1,
                    missing: Some(slate_size),
                },
            );
        }
    }
    map
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use lumit_media::encode::VideoCodec;

    /// A 30 fps, 5 s solid comp — the smallest document a real export can run
    /// against (mirrors the headless tests' builder; modules cannot share test
    /// helpers without exporting them, and exporting a test helper is worse).
    fn solid_doc(w: u32, h: u32) -> (Arc<Document>, Uuid) {
        use lumit_core::model::{
            Composition, LayerKind, LinearColour, ProjectItem, SolidDef, Switches,
        };
        use lumit_core::time::{CompTime, Duration as CompDuration, FrameRate, Rational};
        let mut doc = Document::new();
        let solid_id = Uuid::now_v7();
        doc.items.push(ProjectItem::Solid(SolidDef {
            id: solid_id,
            name: "Solid".into(),
            colour: LinearColour([0.9, 0.2, 0.1, 1.0]),
            width: w,
            height: h,
            extra: serde_json::Map::new(),
        }));
        let comp_id = Uuid::now_v7();
        doc.items.push(ProjectItem::Composition(Composition {
            graph: None,
            master_volume_db: 0.0,
            sound_mix: false,
            groups: Vec::new(),
            beat_grid: None,
            id: comp_id,
            name: "Scene".into(),
            width: w,
            height: h,
            frame_rate: FrameRate::new(30, 1).unwrap(),
            duration: CompDuration(Rational::new(5, 1).unwrap()),
            background: LinearColour::BLACK,
            work_area: None,
            layers: vec![lumit_core::model::Layer {
                graph: Default::default(),
                markers: Vec::new(),
                id: Uuid::now_v7(),
                name: "Solid".into(),
                kind: LayerKind::Solid { def: solid_id },
                in_point: CompTime(Rational::new(0, 1).unwrap()),
                out_point: CompTime(Rational::new(5, 1).unwrap()),
                start_offset: CompTime(Rational::new(0, 1).unwrap()),
                transform: Default::default(),
                matte: None,
                parent: None,
                label: 0,
                volume_db: lumit_core::anim::Property::zero(),
                pan: lumit_core::anim::Property::zero(),
                audio_only: false,
                adjustment: false,
                retime: None,
                interpolation: Default::default(),
                parked_flow: None,
                graph_inputs: None,
                blend: Default::default(),
                masks: Vec::new(),
                paint: Vec::new(),
                puppet: None,
                effects: Vec::new(),
                styles: Vec::new(),
                switches: Switches::default(),
                extra: serde_json::Map::new(),
            }],
            markers: Vec::new(),
            motion_blur: lumit_core::model::MotionBlur::default(),
            extra: serde_json::Map::new(),
        }));
        (Arc::new(doc), comp_id)
    }

    /// [`solid_doc`] with a top-to-bottom Gradient over it: a smooth ramp in
    /// scene-linear float, which is the only kind of picture that can show
    /// whether a sixteen-bit export is really sixteen bits.
    fn gradient_doc(w: u32, h: u32) -> (Arc<Document>, Uuid) {
        let (doc, comp_id) = solid_doc(w, h);
        let mut doc = Document::clone(&doc);
        let mut fx = lumit_core::fx::instantiate("gradient").expect("gradient is a built-in");
        for p in &mut fx.params {
            let set = match p.id.as_str() {
                // White at the top row to black at the bottom, straight down.
                "start_x" | "start_y" | "end_x" => 0.0,
                "end_y" => f64::from(h),
                _ => continue,
            };
            p.value = lumit_core::model::EffectValue::Float(lumit_core::anim::Property::fixed(set));
        }
        for item in &mut doc.items {
            if let ProjectItem::Composition(comp) = item {
                if comp.id == comp_id {
                    comp.layers[0].effects.push(fx.clone());
                }
            }
        }
        (Arc::new(doc), comp_id)
    }

    fn spec(format: ExportFormat, w: u32, h: u32) -> ExportSpec {
        ExportSpec {
            format,
            target: Some((w, h)),
            include_audio: false,
            ..ExportSpec::default()
        }
    }

    /// Run an export to completion on this thread, skipping (Ok(None)) on a
    /// machine with no GPU adapter — the lavapipe convention.
    fn run_now(
        doc: &Arc<Document>,
        comp: Uuid,
        path: &std::path::Path,
        spec: &ExportSpec,
    ) -> Option<Result<(), String>> {
        let (tx, _rx) = channel();
        let cancel = AtomicBool::new(false);
        match run(doc, comp, &[], path, spec, &tx, &cancel) {
            Err(e) if e.starts_with("export renderer:") => {
                lumit_gpu::no_adapter();
                None
            }
            other => Some(other),
        }
    }

    /// The chosen rate is stamped as an exact rational, never a rounded whole
    /// number — 29.97 must not quietly become 30 (docs/impl/rational-time).
    #[test]
    fn fps_rational_keeps_fractional_rates_exact() {
        assert_eq!(fps_rational(60.0), (60, 1));
        assert_eq!(fps_rational(29.97), (2997, 100));
        assert_eq!(fps_rational(23.976), (2997, 125));
        assert_eq!(fps_rational(0.0), (1, 1), "clamped, never zero");
    }

    /// An explicit range exports exactly its frames — here comp frames 10..20
    /// as a PNG sequence, so the file count *is* the assertion.
    #[test]
    fn an_explicit_range_exports_exactly_its_frames_as_stills() {
        let (doc, comp) = solid_doc(32, 16);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.png");
        let mut sp = spec(
            ExportFormat::Images(lumit_media::encode::ImageFormat::Png),
            32,
            16,
        );
        sp.range = Some((10, 20));
        let Some(result) = run_now(&doc, comp, &path, &sp) else {
            return;
        };
        result.expect("export runs");
        for n in 1..=10 {
            assert!(
                lumit_media::encode::sequence_frame_path(&path, n).exists(),
                "frame {n} missing"
            );
        }
        assert!(
            !lumit_media::encode::sequence_frame_path(&path, 11).exists(),
            "ten frames were asked for, ten written"
        );
    }

    /// Volume baking (docs/09 §6): a static Volume is exactly its constant
    /// gain; a keyframed fade becomes a control-rate envelope sampled in
    /// layer time (comp time − start offset), falling to true zero at the
    /// −inf knee.
    #[test]
    fn volume_bake_static_gain_and_animated_envelope() {
        use lumit_core::anim::{Animation, Keyframe, Property, SideInterp};
        use lumit_core::Rational;
        let job = |volume: Property, offset_s: f64| AudioJob {
            item: uuid::Uuid::nil(),
            layer: uuid::Uuid::nil(),
            clip: None,
            path: PathBuf::new(),
            in_s: 0.0,
            out_s: 10.0,
            offset_s,
            volume,
            pan: Property::zero(),
            carriers: Vec::new(),
            fade: None,
            driven: None,
            chain: None,
            clip_chain: None,
        };
        let (g, env) = volume_bake(&job(Property::fixed(-6.0), 0.0), 0, 48_000, 48_000);
        assert!(env.is_none(), "static volume needs no envelope");
        assert!((g[0] - 0.501_19).abs() < 1e-3);
        assert!((g[1] - g[0]).abs() < 1e-9, "centred: both channels alike");

        // A 1 s fade 0 dB → −inf, on a layer whose time 0 sits at comp 1 s.
        let key = |t: i64, v: f64| Keyframe {
            time: Rational::new(t, 1).unwrap(),
            value: v,
            interp_in: SideInterp::Linear,
            interp_out: SideInterp::Linear,
        };
        let fade = Property {
            animation: Animation::Keyframed(vec![key(0, 0.0), key(1, -100.0)]),
            extra: serde_json::Map::new(),
        };
        // Placed at comp 1 s (start_frame 48000), offset 1 s: layer time 0..1.
        let (g, env) = volume_bake(&job(fade.clone(), 1.0), 48_000, 48_000, 48_000);
        assert_eq!(g, [1.0, 1.0]);
        let env = env.unwrap();
        assert!(
            (env.gain_at(0)[0] - 1.0).abs() < 1e-6,
            "fade starts at unity"
        );
        assert!(
            env.gain_at(0)[0] > env.gain_at(24_000)[0]
                && env.gain_at(24_000)[0] > env.gain_at(47_500)[0],
            "the fade descends"
        );
        assert_eq!(
            env.gain_at(48_000),
            [0.0, 0.0],
            "the −inf knee lands at silence"
        );

        // Carrier chain (precomp audio): the precomp layer's −6 dB multiplies
        // the inner layer's −6 dB — two static links stay a constant product.
        let mut carried = job(Property::fixed(-6.0), 0.0);
        carried.carriers = vec![Carrier {
            volume: Property::fixed(-6.0),
            pan: Property::zero(),
            offset_s: 0.0,
            fade: None,
            chain: None,
        }];
        let (g, env) = volume_bake(&carried, 0, 48_000, 48_000);
        assert!(env.is_none());
        assert!(
            (g[0] - 0.251_19).abs() < 1e-3,
            "gains multiply through the chain"
        );
        // An animated carrier envelopes the whole chain.
        let mut fading_carrier = job(Property::fixed(0.0), 0.0);
        fading_carrier.carriers = vec![Carrier {
            volume: fade,
            pan: Property::zero(),
            offset_s: 0.0,
            fade: None,
            chain: None,
        }];
        let (_, env) = volume_bake(&fading_carrier, 0, 48_000, 48_000);
        assert!(env.is_some(), "an animated carrier forces the envelope");

        // **Pan is the same stage**: a hard-right balance silences the
        // left channel and lifts the right by the constant-power √2, and a
        // Precomp layer's own balance multiplies channel by channel.
        let mut right = job(Property::fixed(0.0), 0.0);
        right.pan = Property::fixed(lumit_audio::mix::PAN_FULL);
        let (g, env) = volume_bake(&right, 0, 48_000, 48_000);
        assert!(env.is_none(), "a static pan needs no envelope either");
        assert!(g[0].abs() < 1e-6, "nothing left of a hard-right layer");
        assert!((g[1] - std::f32::consts::SQRT_2).abs() < 1e-5);

        let mut opposed = job(Property::fixed(0.0), 0.0);
        opposed.pan = Property::fixed(-lumit_audio::mix::PAN_FULL);
        opposed.carriers = vec![Carrier {
            volume: Property::zero(),
            pan: Property::fixed(lumit_audio::mix::PAN_FULL),
            offset_s: 0.0,
            fade: None,
            chain: None,
        }];
        let (g, _) = volume_bake(&opposed, 0, 48_000, 48_000);
        assert!(
            g[0].abs() < 1e-6 && g[1].abs() < 1e-6,
            "hard left inside a hard-right precomp leaves nothing on either side"
        );

        // An animated pan forces the envelope even with a static Volume, and
        // the two channels then move independently.
        let mut sweeping = job(Property::fixed(0.0), 0.0);
        sweeping.pan = Property {
            animation: Animation::Keyframed(vec![key(0, -100.0), key(1, 100.0)]),
            extra: serde_json::Map::new(),
        };
        let (_, env) = volume_bake(&sweeping, 0, 48_000, 48_000);
        let env = env.expect("an animated pan forces the envelope");
        assert!(env.gain_at(0)[0] > env.gain_at(0)[1], "starts on the left");
        assert!(
            env.gain_at(47_000)[1] > env.gain_at(47_000)[0],
            "and ends on the right"
        );
    }

    /// A **clip crossfade** is two opposed equal-power ramps: each
    /// clip's own envelope, so sliding one moves its ramp with it, and the
    /// pair holds level across the join rather than dipping in the middle the
    /// way a straight line would.
    #[test]
    fn a_clip_join_is_two_opposed_equal_power_ramps() {
        // Clip A runs 0..2 and fades out over its last second; clip B runs
        // 1..3 and fades in over its first.
        let out = ClipFade {
            start_s: 0.0,
            head_s: 0.0,
            end_s: 2.0,
            tail_s: 1.0,
            ..Default::default()
        };
        let into = ClipFade {
            start_s: 1.0,
            head_s: 1.0,
            end_s: 3.0,
            tail_s: 0.0,
            ..Default::default()
        };
        assert!(out.is_active() && into.is_active());
        assert!((out.gain_at(0.5) - 1.0).abs() < 1e-6, "before the join");
        assert!((into.gain_at(2.5) - 1.0).abs() < 1e-6, "after it");
        assert!(out.gain_at(2.0).abs() < 1e-6, "A is gone by its end");
        assert!(into.gain_at(1.0).abs() < 1e-6, "B starts from nothing");

        // Across the overlap the two powers sum to one at every point, which
        // is what "equal power" buys: no dip in the middle of the dissolve.
        for n in 0..=10 {
            let t = 1.0 + f64::from(n) / 10.0;
            let (a, b) = (out.gain_at(t), into.gain_at(t));
            assert!(
                (a * a + b * b - 1.0).abs() < 1e-5,
                "t={t}: {a}² + {b}² is not one"
            );
        }
        // A clip with no neighbour has no ramp and costs nothing.
        assert!(!ClipFade::default().is_active());
        assert_eq!(ClipFade::default().gain_at(5.0), 1.0);
    }

    /// **Preview and export hear the same mix**, with a pan sweep and
    /// a clip crossfade ramp on it — the two later additions to the gain
    /// stage.
    ///
    /// One job, one bake, and then the two mixers that exist: the exporter's
    /// `mix_stereo_at` over a `PlacedAudio`, and playback's `MixPlan` over the
    /// `PlacedClip` the bridge builds from the same bake. They are different
    /// loops — one writes a whole buffer, one answers a frame at a time from
    /// the realtime callback — so "they agree" is a claim that has to be
    /// checked rather than assumed.
    #[test]
    fn a_panned_and_faded_clip_mixes_the_same_in_both_paths() {
        use lumit_core::anim::{Animation, Keyframe, Property, SideInterp};
        use lumit_core::Rational;
        use std::sync::Arc;

        let rate = 48_000u32;
        let frames = rate as usize; // one second
                                    // A source that is not symmetrical, so a pan that swapped the channels
                                    // would be caught rather than looking identical.
        let samples: Vec<f32> = (0..frames)
            .flat_map(|n| {
                let t = n as f32 / frames as f32;
                [0.6 * (t * 9.0).sin(), 0.4 * (t * 5.0).cos()]
            })
            .collect();

        let key = |t: i64, v: f64| Keyframe {
            time: Rational::new(t, 1).expect("time"),
            value: v,
            interp_in: SideInterp::Linear,
            interp_out: SideInterp::Linear,
        };
        let job = AudioJob {
            item: uuid::Uuid::nil(),
            layer: uuid::Uuid::nil(),
            clip: None,
            path: PathBuf::new(),
            in_s: 0.0,
            out_s: 1.0,
            offset_s: 0.0,
            volume: Property::fixed(-3.0),
            // Sweeping hard left to hard right across the second.
            pan: Property {
                animation: Animation::Keyframed(vec![key(0, -100.0), key(1, 100.0)]),
                extra: serde_json::Map::new(),
            },
            carriers: Vec::new(),
            // And rising out of a join over its first quarter second.
            fade: Some(ClipFade {
                start_s: 0.0,
                head_s: 0.25,
                end_s: 1.0,
                tail_s: 0.0,
                ..Default::default()
            }),
            driven: None,
            chain: None,
            clip_chain: None,
        };

        let (gain, envelope) = volume_bake(&job, 0, frames, rate);
        let envelope = envelope.expect("a sweep and a ramp both force the envelope");
        let baked = lumit_audio::mix::mix_stereo_at(
            &[lumit_audio::mix::PlacedAudio {
                start_frame: 0,
                samples: &samples,
                gain,
                envelope: Some(envelope.clone()),
            }],
            frames,
            1.0,
        );
        let plan = lumit_audio::mix::MixPlan {
            clips: vec![lumit_audio::mix::PlacedClip {
                source: Source::whole(Arc::new(lumit_media::AudioBuffer {
                    rate,
                    samples: samples.clone(),
                })),
                start_frame: 0,
                src_start: 0,
                len: frames,
                gain,
                envelope: Some(Arc::new(envelope)),
                meter: 0,
            }],
            total_frames: frames,
            master_gain: 1.0,
        };
        for i in (0..frames).step_by(97) {
            let (l, r) = plan.frame_at(i);
            assert!(
                (l - baked[i * 2]).abs() < 1e-6 && (r - baked[i * 2 + 1]).abs() < 1e-6,
                "frame {i}: playback ({l}, {r}) vs export ({}, {})",
                baked[i * 2],
                baked[i * 2 + 1]
            );
        }

        // And the mix really is doing both things, so the agreement above is
        // not two paths agreeing on having ignored them.
        assert!(
            baked[0].abs() < 1e-4 && baked[1].abs() < 1e-4,
            "the head ramp starts from silence"
        );
        let quarter = frames / 4;
        let (l0, r0) = plan.frame_at(quarter);
        let (l1, r1) = plan.frame_at(frames - 2);
        assert!(
            l0.abs() > r0.abs() && r1.abs() > l1.abs(),
            "the sweep starts on the left and ends on the right"
        );
    }

    /// The A/V interleave rule: cumulative rounding never drifts, and the
    /// total after all frames equals the whole soundtrack.
    #[test]
    fn audio_samples_through_never_drifts() {
        let (fps, rate) = (60.0, 48_000u32);
        // 60 fps at 48 kHz is exactly 800 samples per frame.
        assert_eq!(audio_samples_through(1, fps, rate), 800);
        assert_eq!(audio_samples_through(300, fps, rate), 240_000);
        // An awkward rate: 29.97 fps. Per-frame chunks vary by ±1 sample but
        // the cumulative total stays glued to the exact value.
        let fps = 30_000.0 / 1001.0;
        let mut prev = 0;
        for n in 1..=1000 {
            let now = audio_samples_through(n, fps, rate);
            let chunk = now - prev;
            assert!((1601..=1602).contains(&chunk), "frame {n} chunk {chunk}");
            let exact = n as f64 / fps * 48_000.0;
            assert!((now as f64 - exact).abs() <= 0.5, "frame {n} drifted");
            prev = now;
        }
        // Degenerate input answers zero, never panics.
        assert_eq!(audio_samples_through(100, 0.0, rate), 0);
    }

    /// A setting the chosen format cannot honour is refused before a frame is
    /// rendered — never silently dropped, which would deliver a file that is
    /// not what was asked for.
    #[test]
    fn a_spec_the_format_cannot_honour_is_refused() {
        use lumit_media::encode::ImageFormat;
        let base = spec(ExportFormat::Video(VideoCodec::H264), 320, 240);
        base.check().expect("the plain case runs");

        let deep = ExportSpec {
            depth: BitDepth::Sixteen,
            ..base.clone()
        };
        assert!(deep.check().is_err(), "mp4 cannot carry 16-bit");

        let transparent = ExportSpec {
            channels: Channels::RgbAlpha,
            ..base.clone()
        };
        assert!(transparent.check().is_err(), "mp4 cannot carry alpha");

        // The same two settings are fine on a PNG sequence.
        let stills = ExportSpec {
            format: ExportFormat::Images(ImageFormat::Png),
            depth: BitDepth::Sixteen,
            channels: Channels::RgbAlpha,
            ..base.clone()
        };
        stills.check().expect("a PNG carries both");

        // An OCIO space is refused until OCIO exists: a wrong colour space in
        // a delivered file is worse than an export that did not run.
        let ocio = ExportSpec {
            colour_space: ColourSpace::Ocio("ACES - ACEScg".into()),
            ..base
        };
        assert!(ocio.check().is_err());
        assert!(ColourSpace::SrgbRec709.is_available());
        assert!(!ColourSpace::Ocio("anything".into()).is_available());
    }

    /// The derived Rec.709 RGB→XYZ matrix against the one BT.709 prints. The
    /// matrices are worked out from the published chromaticities rather than
    /// typed in, so this is the check that the derivation — and therefore
    /// every colour value below — is the standard's and not an invention.
    #[test]
    fn the_derived_primaries_matrices_match_the_published_ones() {
        let close = |got: [[f64; 3]; 3], want: [[f64; 3]; 3], tol: f64, what: &str| {
            for r in 0..3 {
                for c in 0..3 {
                    assert!(
                        (got[r][c] - want[r][c]).abs() < tol,
                        "{what}[{r}][{c}]: {} vs published {}",
                        got[r][c],
                        want[r][c]
                    );
                }
            }
        };
        // ITU-R BT.709-6, §1.4.1 (the four-figure matrix it prints).
        close(
            rgb_to_xyz(&REC709_PRIMARIES),
            [
                [0.4124, 0.3576, 0.1805],
                [0.2126, 0.7152, 0.0722],
                [0.0193, 0.1192, 0.9505],
            ],
            5e-4,
            "Rec.709 RGB→XYZ",
        );
        // ITU-R BT.2020-2.
        close(
            rgb_to_xyz(&REC2020_PRIMARIES),
            [
                [0.6370, 0.1446, 0.1689],
                [0.2627, 0.6780, 0.0593],
                [0.0000, 0.0281, 1.0610],
            ],
            5e-4,
            "Rec.2020 RGB→XYZ",
        );
        // SMPTE EG 432-1 P3-D65, as the ICC "Display P3" profile publishes it.
        close(
            rgb_to_xyz(&DISPLAY_P3_PRIMARIES),
            [
                [0.4866, 0.2657, 0.1982],
                [0.2290, 0.6917, 0.0793],
                [0.0000, 0.0451, 1.0439],
            ],
            5e-4,
            "Display P3 RGB→XYZ",
        );
        // And the composed 709→2020 matrix against ITU-R BT.2087-0's own.
        close(
            primaries_change(&REC709_PRIMARIES, &REC2020_PRIMARIES),
            [
                [0.6274, 0.3293, 0.0433],
                [0.0691, 0.9195, 0.0114],
                [0.0164, 0.0880, 0.8956],
            ],
            5e-4,
            "Rec.709→Rec.2020",
        );
    }

    /// A whole space transform, end to end, against values worked by hand.
    #[test]
    fn each_colour_space_transforms_known_values() {
        // The default is a pass-through — no transform object at all, so an
        // export naming it is byte-for-byte the export it always was.
        assert!(ColourSpace::SrgbRec709.transform().is_none());
        assert!(ColourSpace::Ocio("x".into()).transform().is_none());

        // sRGB 0.5 is 0.2140 linear (IEC 61966-2-1: ((0.5+0.055)/1.055)^2.4).
        let lin = ColourSpace::Linear.transform().unwrap();
        let out = lin.apply([0.5, 0.5, 0.5]);
        for v in out {
            assert!((v - 0.214_041).abs() < 1e-5, "linear: {v}");
        }
        // The same light through BT.709's OETF: 1.099·0.214041^0.45 − 0.099.
        let r709 = ColourSpace::Rec709.transform().unwrap();
        for v in r709.apply([0.5, 0.5, 0.5]) {
            assert!((v - 0.450_189).abs() < 1e-5, "rec709: {v}");
        }

        // White and black survive every space: the primaries matrices take
        // D65 white to D65 white by construction, which is the whole point of
        // the scaling step in `rgb_to_xyz`.
        for space in BUILT_IN_COLOUR_SPACES {
            let Some(t) = space.transform() else { continue };
            for v in t.apply([1.0, 1.0, 1.0]) {
                assert!((v - 1.0).abs() < 1e-6, "{} moved white", space.label());
            }
            for v in t.apply([0.0, 0.0, 0.0]) {
                assert!(v.abs() < 1e-9, "{} moved black", space.label());
            }
            // Grey stays grey — all three channels equal — in every space.
            let g = t.apply([0.5, 0.5, 0.5]);
            assert!(
                (g[0] - g[1]).abs() < 1e-9 && (g[1] - g[2]).abs() < 1e-9,
                "{} tinted a neutral: {g:?}",
                space.label()
            );
        }

        // Saturated Rec.709 red is inside Rec.2020's gamut, so it becomes a
        // *less* saturated 2020 triple — some green and blue appear, and the
        // red drops. (BT.2087's first column: 0.6274, 0.0691, 0.0164.)
        let r2020 = ColourSpace::Rec2020.transform().unwrap();
        let red = r2020.apply([1.0, 0.0, 0.0]);
        assert!(red[0] < 1.0 && red[0] > 0.75, "2020 red: {red:?}");
        assert!(red[1] > 0.0 && red[1] < red[0], "2020 red: {red:?}");
        assert!(red[2] > 0.0 && red[2] < red[1], "2020 red: {red:?}");

        // Display P3 is wider than 709 but narrower than 2020, so the same
        // red lands between the two.
        let p3 = ColourSpace::DisplayP3.transform().unwrap();
        let p3_red = p3.apply([1.0, 0.0, 0.0]);
        assert!(
            p3_red[0] > red[0],
            "P3 is narrower than 2020, so 709 red should stay redder in it: {p3_red:?} vs {red:?}"
        );
        assert!(p3_red[0] < 1.0, "P3 red: {p3_red:?}");

        // Deterministic: the same input gives the same bits, every time.
        assert_eq!(r2020.apply([0.3, 0.6, 0.9]), r2020.apply([0.3, 0.6, 0.9]));
    }

    /// Crop arithmetic, in pixels at composition size: the size it
    /// leaves, the window it keeps, and the pixels it actually copies.
    #[test]
    fn crop_maths_keeps_the_window_it_says_it_keeps() {
        let crop = Crop {
            top: 1,
            left: 2,
            bottom: 3,
            right: 4,
        };
        assert_eq!(crop.output_size(10, 10), (4, 6));
        assert_eq!(crop.window(10, 10), (2, 1, 4, 6));
        assert!(Crop::NONE.is_none());
        assert_eq!(Crop::NONE.output_size(10, 10), (10, 10));

        // Insets that meet leave one pixel rather than none — a slip of the
        // fingers is not a reason to fail an export.
        let silly = Crop {
            top: 99,
            left: 99,
            bottom: 99,
            right: 99,
        };
        assert_eq!(silly.output_size(10, 10), (1, 1));
        let (x, y, w, h) = silly.window(10, 10);
        assert_eq!((w, h), (1, 1));
        assert!(x < 10 && y < 10, "the window stays inside the frame");

        // The pixels: a 4×3 frame of one byte per pixel, numbered by position.
        let frame: Vec<u8> = (0..12).collect();
        let one_off_each_side = Crop {
            top: 1,
            left: 1,
            bottom: 1,
            right: 1,
        };
        assert_eq!(one_off_each_side.apply(&frame, 4, 3, 1), vec![5, 6]);
        // Four bytes a pixel, the real shape.
        let rgba: Vec<u8> = (0..(4 * 2 * 2)).collect();
        let right_half = Crop {
            top: 0,
            left: 1,
            bottom: 0,
            right: 0,
        };
        assert_eq!(
            right_half.apply(&rgba, 2, 2, 4),
            vec![4, 5, 6, 7, 12, 13, 14, 15]
        );
        // No crop is the frame itself, and a buffer too small comes back
        // whole rather than panicking mid-export.
        assert_eq!(Crop::NONE.apply(&frame, 4, 3, 1), frame);
        assert_eq!(one_off_each_side.apply(&[1, 2, 3], 4, 3, 1), vec![1, 2, 3]);
        // Regression: a zero-sized frame used to index past an empty buffer.
        assert!(one_off_each_side.apply::<u8>(&[], 0, 0, 4).is_empty());
        assert!(one_off_each_side.apply::<u8>(&[], 4, 0, 4).is_empty());
    }

    /// The pack stage: what each channel/alpha choice does to the finished
    /// pixels, on the CPU, without a graphics card in sight.
    #[test]
    fn the_pack_stage_writes_what_each_choice_asks_for() {
        // One half-covered premultiplied pixel and one opaque one.
        let src = [100u8, 50, 0, 128, 10, 20, 30, 255];

        // RGB: alpha forced opaque, colour untouched.
        let rgb = pack_frame(&src, Channels::Rgb, AlphaMode::Premultiplied, None);
        assert_eq!(rgb, [100, 50, 0, 255, 10, 20, 30, 255]);

        // Premultiplied RGBA: exactly what the compositor produced.
        let pre = pack_frame(&src, Channels::RgbAlpha, AlphaMode::Premultiplied, None);
        assert_eq!(pre, src);

        // Straight RGBA: colour divided back up by its coverage.
        let straight = pack_frame(&src, Channels::RgbAlpha, AlphaMode::Straight, None);
        assert_eq!(straight[3], 128, "coverage itself is unchanged");
        assert_eq!(straight[0], 199, "100 / (128/255) rounds to 199");
        assert_eq!(straight[1], 100);
        assert_eq!(&straight[4..], &src[4..], "an opaque pixel is untouched");

        // A colour beyond its own coverage clamps rather than overflowing,
        // and zero coverage has no colour to recover.
        let odd = [200u8, 0, 0, 100, 9, 9, 9, 0];
        let straight = pack_frame(&odd, Channels::RgbAlpha, AlphaMode::Straight, None);
        assert_eq!(straight[0], 255);
        assert_eq!(&straight[4..], &[0, 0, 0, 0]);

        // Sixteen bits: the same rules on sixteen-bit input, written
        // little-endian and NOT widened from anything — the codes the deep
        // read-back gave are the codes the file gets.
        let deep = [40_000u16, 500, 0, 32_768, 1, 2, 3, 65_535];
        let wide = pack_frame(&deep, Channels::RgbAlpha, AlphaMode::Premultiplied, None);
        assert_eq!(wide.len(), deep.len() * 2);
        let samples: Vec<u16> = wide
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(samples, deep, "premultiplied sixteen-bit is a copy");

        // Straight alpha at sixteen bits divides by sixteen-bit full scale,
        // and a value past its own coverage still clamps.
        let straight16 = pack_frame(&deep, Channels::RgbAlpha, AlphaMode::Straight, None);
        let first: Vec<u16> = straight16[..8]
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(first[0], 65_535, "40000 over half coverage clamps to full");
        assert_eq!(first[1], 1_000, "500 over half coverage doubles");
        assert_eq!(first[3], 32_768, "coverage itself is unchanged");
        // A sixteen-bit frame carries values eight bits cannot hold at all:
        // the odd codes here survive, where the old widened path could only
        // ever have written multiples of 257.
        assert!(
            samples.iter().any(|v| v % 257 != 0),
            "nothing here is a stretched byte"
        );
    }

    /// Auto versus manual is stored in the settings and resolved at the last
    /// moment, against the frame actually being written.
    #[test]
    fn the_bitrate_choice_resolves_auto_manual_and_neither() {
        use lumit_media::encode::ImageFormat;
        let auto = spec(ExportFormat::Video(VideoCodec::H264), 1920, 1080);
        assert_eq!(auto.bitrate, Bitrate::Auto, "auto is the default");
        assert_eq!(
            auto.resolved_bitrate((1920, 1080), 60.0),
            Some((16_000_000, Some(24_000_000)))
        );

        // A typed number with no peak takes the 1.5× fallback.
        let manual = ExportSpec {
            bitrate: Bitrate::Manual {
                target_bps: 10_000_000,
                peak_bps: None,
            },
            ..auto.clone()
        };
        assert_eq!(
            manual.resolved_bitrate((1920, 1080), 60.0),
            Some((10_000_000, Some(15_000_000)))
        );

        // No target named: the composition's own size decides the auto rate.
        let comp_sized = ExportSpec {
            target: None,
            ..auto.clone()
        };
        assert_eq!(
            comp_sized.resolved_bitrate((1280, 720), 30.0),
            Some((4_000_000, Some(6_000_000)))
        );

        // Lossless and audio-only formats have no video bitrate at all.
        let stills = ExportSpec {
            format: ExportFormat::Images(ImageFormat::Png),
            ..auto.clone()
        };
        assert_eq!(stills.resolved_bitrate((1920, 1080), 60.0), None);
        let sound = ExportSpec {
            format: ExportFormat::Audio(AudioFormat::M4a),
            ..auto
        };
        assert_eq!(sound.resolved_bitrate((1920, 1080), 60.0), None);
    }

    /// The document-shaped render options act on the export's own snapshot,
    /// through nested comps, and leave the original untouched.
    #[test]
    fn render_overrides_clear_fx_and_solo_everywhere_or_nothing_at_all() {
        let (doc, comp_id) = solid_doc(32, 16);
        // Give the one layer both switches something to clear.
        let mut seeded = Document::clone(&doc);
        for item in &mut seeded.items {
            if let ProjectItem::Composition(c) = item {
                for l in &mut c.layers {
                    l.switches.fx = true;
                    l.switches.solo = true;
                }
            }
        }
        let seeded = Arc::new(seeded);

        // Defaults change nothing, and say so by answering None rather than
        // cloning a whole document to alter nothing.
        assert!(apply_render_overrides(&seeded, &RenderOptions::default()).is_none());

        let off = RenderOptions {
            effects: false,
            honour_solo: false,
            ..RenderOptions::default()
        };
        let patched = apply_render_overrides(&seeded, &off).expect("something changed");
        let layer = &patched.comp(comp_id).unwrap().layers[0];
        assert!(!layer.switches.fx, "effects off clears the fx switch");
        assert!(!layer.switches.solo, "solo ignored clears the solo switch");
        // The snapshot the export was handed is untouched.
        let original = &seeded.comp(comp_id).unwrap().layers[0];
        assert!(original.switches.fx && original.switches.solo);

        // Each half acts on its own.
        let fx_only = RenderOptions {
            effects: false,
            ..RenderOptions::default()
        };
        let patched = apply_render_overrides(&seeded, &fx_only).unwrap();
        let layer = &patched.comp(comp_id).unwrap().layers[0];
        assert!(!layer.switches.fx && layer.switches.solo);
    }

    /// A two-level document for the guide-layer tests: an outer comp holding
    /// a (non-collapsed) Precomp of [`solid_doc`]'s comp plus a guide layer of
    /// its own, and a second guide layer inside the nested comp. Answers the
    /// outer comp, the outer guide layer and the nested one.
    fn nested_guide_doc() -> (Arc<Document>, Uuid, Uuid, Uuid) {
        use lumit_core::model::{Composition, LayerKind, LinearColour};
        use lumit_core::time::{Duration as CompDuration, FrameRate, Rational};
        let (doc, inner_id) = solid_doc(32, 16);
        let mut doc = Document::clone(&doc);

        // The nested comp gains a guide layer above its solid.
        let inner_guide = Uuid::now_v7();
        // Half-opaque, so a guide layer over the whole frame does not occlude
        // what is under it — the occlusion cull would hide the nested comp
        // from the draw list and the test would prove nothing.
        let mut template = doc.comp(inner_id).unwrap().layers[0].clone();
        template.transform.opacity = lumit_core::anim::Property::fixed(50.0);
        let template = template;
        for item in &mut doc.items {
            if let ProjectItem::Composition(c) = item {
                if c.id == inner_id {
                    let mut g = template.clone();
                    g.id = inner_guide;
                    g.name = "Inner guide".into();
                    g.switches.guide = true;
                    c.layers.insert(0, g);
                }
            }
        }

        // The outer comp: a guide layer over the nested comp.
        let outer_guide = Uuid::now_v7();
        let mut precomp = template.clone();
        precomp.transform.opacity = lumit_core::anim::Property::fixed(100.0);
        precomp.id = Uuid::now_v7();
        precomp.name = "Nested".into();
        precomp.kind = LayerKind::Precomp { comp: inner_id };
        let mut guide = template.clone();
        guide.id = outer_guide;
        guide.name = "Outer guide".into();
        guide.switches.guide = true;
        let outer_id = Uuid::now_v7();
        doc.items.push(ProjectItem::Composition(Composition {
            graph: None,
            master_volume_db: 0.0,
            sound_mix: false,
            groups: Vec::new(),
            beat_grid: None,
            id: outer_id,
            name: "Outer".into(),
            width: 32,
            height: 16,
            frame_rate: FrameRate::new(30, 1).unwrap(),
            duration: CompDuration(Rational::new(5, 1).unwrap()),
            background: LinearColour::BLACK,
            work_area: None,
            layers: vec![guide, precomp],
            markers: Vec::new(),
            motion_blur: lumit_core::model::MotionBlur::default(),
            extra: serde_json::Map::new(),
        }));
        (Arc::new(doc), outer_id, outer_guide, inner_guide)
    }

    /// The nested comp of [`nested_guide_doc`].
    fn nested_comp_of(doc: &Arc<Document>, outer: Uuid) -> Uuid {
        doc.comp(outer)
            .unwrap()
            .layers
            .iter()
            .find_map(|l| match l.kind {
                lumit_core::model::LayerKind::Precomp { comp } => Some(comp),
                _ => None,
            })
            .unwrap()
    }

    /// A two-level document for the motion-blur override: an outer comp
    /// holding a Precomp of [`solid_doc`]'s comp, both comp masters off, and
    /// the solid inside the nested comp **checked** for blur. Moving, so the
    /// sub-frame samples are genuinely different placements rather than the
    /// same one sixteen times. Answers the outer comp id and the nested one.
    fn checked_blur_doc() -> (Arc<Document>, Uuid, Uuid) {
        use lumit_core::anim::{Animation, Keyframe, Property, SideInterp};
        use lumit_core::model::{Composition, LayerKind, LinearColour};
        use lumit_core::time::{Duration as CompDuration, FrameRate, Rational};
        let (doc, inner_id) = solid_doc(32, 16);
        let mut doc = Document::clone(&doc);

        let mut template = doc.comp(inner_id).unwrap().layers[0].clone();
        for item in &mut doc.items {
            if let ProjectItem::Composition(c) = item {
                if c.id == inner_id {
                    for l in &mut c.layers {
                        l.switches.motion_blur = true;
                        // A layer standing still smears into itself, which no
                        // test could tell from not smearing at all.
                        let key = |t: i64, value: f64| Keyframe {
                            time: Rational::new(t, 1).unwrap(),
                            value,
                            interp_in: SideInterp::Linear,
                            interp_out: SideInterp::Linear,
                        };
                        l.transform.position_x = Property {
                            animation: Animation::Keyframed(vec![key(0, 0.0), key(5, 400.0)]),
                            extra: serde_json::Map::new(),
                        };
                    }
                    template = c.layers[0].clone();
                }
            }
        }

        let mut precomp = template.clone();
        precomp.id = Uuid::now_v7();
        precomp.name = "Nested".into();
        precomp.kind = LayerKind::Precomp { comp: inner_id };
        let outer_id = Uuid::now_v7();
        doc.items.push(ProjectItem::Composition(Composition {
            graph: None,
            master_volume_db: 0.0,
            sound_mix: false,
            groups: Vec::new(),
            beat_grid: None,
            id: outer_id,
            name: "Outer".into(),
            width: 32,
            height: 16,
            frame_rate: FrameRate::new(30, 1).unwrap(),
            duration: CompDuration(Rational::new(5, 1).unwrap()),
            background: LinearColour::BLACK,
            work_area: None,
            layers: vec![precomp],
            markers: Vec::new(),
            // Off, like the nested one: the point of *On for checked layers*
            // is that it reaches a master that is off at every depth.
            motion_blur: lumit_core::model::MotionBlur::default(),
            extra: serde_json::Map::new(),
        }));
        (Arc::new(doc), outer_id, inner_id)
    }

    /// The blur the one shared helper works out for a comp and a
    /// layer: what the preview draws and what the export draws, from the same
    /// call, so this test reads the picture rather than the switches.
    fn blur_samples(doc: &Document, comp_id: Uuid) -> usize {
        let comp = doc.comp(comp_id).unwrap();
        let context = Arc::new(lumit_core::expression::ExpressionContext::detached());
        crate::build::motion_blur_samples(comp, &comp.layers[0], 0.5, context).len()
    }

    /// *Motion blur* is the export's own answer, at every depth: *on for
    /// checked layers* turns the master on in every comp in the walk and
    /// leaves the checks alone, *off for all layers* shuts both, and *current
    /// settings* copies nothing at all.
    #[test]
    fn the_motion_blur_override_reaches_every_comp_in_the_walk() {
        let (doc, outer_id, inner_id) = checked_blur_doc();

        // Current settings is the default and changes nothing: no clone, and
        // the checked layer does not smear because its comp master is off.
        assert!(
            apply_render_overrides(&doc, &RenderOptions::default()).is_none(),
            "the comp's own setting is passthrough, so nothing is copied"
        );
        assert_eq!(blur_samples(&doc, inner_id), 0);

        let on = RenderOptions {
            motion_blur: MotionBlurOverride::OnForChecked,
            ..RenderOptions::default()
        };
        let delivery = apply_render_overrides(&doc, &on).expect("the masters change");
        for comp in [outer_id, inner_id] {
            assert!(
                delivery.comp(comp).unwrap().motion_blur.enabled,
                "the master goes on in every comp, nested ones included"
            );
        }
        assert!(
            delivery.comp(inner_id).unwrap().layers[0]
                .switches
                .motion_blur,
            "the per-layer checks are what the phrase honours, so they stand"
        );
        // The picture, not the switch: the checked layer now smears.
        assert_eq!(blur_samples(&delivery, inner_id), 16);
        // And the snapshot the export was handed is untouched.
        assert!(!doc.comp(inner_id).unwrap().motion_blur.enabled);

        // Off for all layers, against a document where the master IS on.
        let mut seeded = Document::clone(&delivery);
        for item in &mut seeded.items {
            if let ProjectItem::Composition(c) = item {
                c.motion_blur.enabled = true;
            }
        }
        let seeded = Arc::new(seeded);
        assert_eq!(blur_samples(&seeded, inner_id), 16);
        let off = RenderOptions {
            motion_blur: MotionBlurOverride::OffForAll,
            ..RenderOptions::default()
        };
        let delivery = apply_render_overrides(&seeded, &off).expect("the masters change back");
        for comp in [outer_id, inner_id] {
            assert!(!delivery.comp(comp).unwrap().motion_blur.enabled);
        }
        assert!(
            !delivery.comp(inner_id).unwrap().layers[0]
                .switches
                .motion_blur,
            "*for all layers* clears the checks too, not just the one gate"
        );
        assert_eq!(blur_samples(&delivery, inner_id), 0);
    }

    /// A comp whose one footage layer blends between source frames: 24fps
    /// media in a 30fps comp, so the moment asked for lands between two frames
    /// the file has. Answers the document, the comp and the probes.
    fn blending_footage_doc() -> (
        Arc<Document>,
        Uuid,
        std::collections::HashMap<Uuid, crate::source::SourceProbe>,
    ) {
        use lumit_core::model::{
            Composition, FootageItem, LayerKind, LinearColour, MediaRef, Switches,
        };
        use lumit_core::time::{Duration as CompDuration, FrameRate, Rational};
        let mut doc = Document::new();
        let item = Uuid::now_v7();
        doc.items.push(ProjectItem::Footage(FootageItem {
            sequence: None,
            id: item,
            name: "shot.mp4".into(),
            media: MediaRef {
                relative_path: "shot.mp4".into(),
                absolute_path: "shot.mp4".into(),
                fingerprint: None,
                extra: serde_json::Map::new(),
            },
            extra: serde_json::Map::new(),
            colour_space: None,
            source_layer: None,
        }));
        let (solid, comp_id) = solid_doc(32, 16);
        let mut layer = solid.comp(comp_id).unwrap().layers[0].clone();
        layer.kind = LayerKind::Footage { item };
        layer.interpolation = Interpolation::Blend;
        layer.switches = Switches::default();
        let comp_id = Uuid::now_v7();
        doc.items.push(ProjectItem::Composition(Composition {
            graph: None,
            master_volume_db: 0.0,
            sound_mix: false,
            groups: Vec::new(),
            beat_grid: None,
            id: comp_id,
            name: "Scene".into(),
            width: 32,
            height: 16,
            frame_rate: FrameRate::new(30, 1).unwrap(),
            duration: CompDuration(Rational::new(5, 1).unwrap()),
            background: LinearColour::BLACK,
            work_area: None,
            layers: vec![layer],
            markers: Vec::new(),
            motion_blur: lumit_core::model::MotionBlur::default(),
            extra: serde_json::Map::new(),
        }));
        let mut probes = std::collections::HashMap::new();
        probes.insert(
            item,
            crate::source::SourceProbe::Video {
                fps: 24.0,
                width: 32,
                height: 16,
                frames: 120,
                audio: false,
            },
        );
        (Arc::new(doc), comp_id, probes)
    }

    /// Whether the decode plan the one walk builds asks for a blended
    /// pair of source frames — the picture Retime blend makes, read where both
    /// the preview and the export read it.
    fn plan_blends(
        doc: &Document,
        comp_id: Uuid,
        probes: &dyn crate::source::SourceProbes,
    ) -> bool {
        let comp = doc.comp(comp_id).unwrap();
        crate::plan::plan_comp_frame(doc, comp, 0.05, crate::plan::Quality::default(), probes)
            .iter()
            .any(|job| job.blend.is_some())
    }

    /// *Retime blend* off falls every layer back to Nearest, and the default
    /// leaves each layer's own policy — and the document — exactly alone.
    #[test]
    fn the_retime_blend_override_stops_the_blend_and_the_default_never_copies() {
        let (doc, comp_id, probes) = blending_footage_doc();

        assert!(
            apply_render_overrides(&doc, &RenderOptions::default()).is_none(),
            "current settings is passthrough, so nothing is copied"
        );
        assert!(
            plan_blends(&doc, comp_id, &probes),
            "the scenario has to blend before the override can stop it"
        );

        let off = RenderOptions {
            retime_blend: RetimeBlendOverride::OffForAll,
            ..RenderOptions::default()
        };
        let delivery = apply_render_overrides(&doc, &off).expect("the policy changes");
        assert_eq!(
            delivery.comp(comp_id).unwrap().layers[0].interpolation,
            Interpolation::Nearest
        );
        assert!(
            !plan_blends(&delivery, comp_id, &probes),
            "off for all layers asks the decoder for whole source frames"
        );
        // The project keeps the policy the editor chose.
        assert_eq!(
            doc.comp(comp_id).unwrap().layers[0].interpolation,
            Interpolation::Blend
        );
    }

    /// Preview equals export with a guide layer present: the file an export
    /// writes is the file it would have written had the guide layer never been
    /// in the document — byte for byte, at both depths.
    #[test]
    fn an_export_writes_the_same_file_as_if_the_guide_layers_were_not_there() {
        let (with_guides, outer_id, outer_guide, inner_guide) = nested_guide_doc();
        let inner_id = nested_comp_of(&with_guides, outer_id);
        let mut without = Document::clone(&with_guides);
        for item in &mut without.items {
            if let ProjectItem::Composition(c) = item {
                c.layers
                    .retain(|l| l.id != outer_guide && l.id != inner_guide);
            }
        }
        assert_eq!(without.comp(inner_id).unwrap().layers.len(), 1);
        let without = Arc::new(without);

        let dir = tempfile::tempdir().unwrap();
        let mut sp = spec(
            ExportFormat::Images(lumit_media::encode::ImageFormat::Png),
            32,
            16,
        );
        sp.range = Some((0, 1));
        let one = dir.path().join("with.png");
        let two = dir.path().join("without.png");
        let Some(first) = run_now(&with_guides, outer_id, &one, &sp) else {
            return;
        };
        first.expect("the guide export runs");
        run_now(&without, outer_id, &two, &sp)
            .expect("an adapter was there a moment ago")
            .expect("the plain export runs");

        let read = |p: &std::path::Path| {
            std::fs::read(lumit_media::encode::sequence_frame_path(p, 1)).unwrap()
        };
        assert_eq!(
            read(&one),
            read(&two),
            "a guide layer changes nothing about the delivered file"
        );
    }

    /// A sound file takes no picture settings (the owner's 2026-08-30 ruling).
    ///
    /// The one that actually reached the file was solo: `honour_solo: false`
    /// clears every layer's solo switch, and the mixer counts solos across
    /// *all* layers, so a picture setting decided what a `.wav`
    /// contained. An audio-only spec now runs at the defaults, so the solos
    /// stand and the mix is the mix.
    #[test]
    fn a_sound_file_takes_no_picture_settings() {
        let (doc, comp_id) = solid_doc(32, 16);
        let mut seeded = Document::clone(&doc);
        for item in &mut seeded.items {
            if let ProjectItem::Composition(c) = item {
                for l in &mut c.layers {
                    l.switches.solo = true;
                }
            }
        }
        let seeded = Arc::new(seeded);

        let picture_settings = RenderOptions {
            effects: false,
            honour_solo: false,
            render_guides: true,
            quality: crate::plan::Quality {
                divisor: 4,
                ..crate::plan::Quality::default()
            },
            ..RenderOptions::default()
        };
        let with_settings = |format| ExportSpec {
            format,
            render: picture_settings,
            ..ExportSpec::default()
        };

        let sound = with_settings(ExportFormat::Audio(AudioFormat::Wav));
        assert_eq!(
            sound.render_options(),
            RenderOptions::default(),
            "a sound file draws nothing, so it takes no drawing settings"
        );
        assert!(
            apply_render_overrides(&seeded, &sound.render_options()).is_none(),
            "so the mix is made from the document as it stands"
        );
        assert!(
            seeded.comp(comp_id).unwrap().layers[0].switches.solo,
            "and the solo the mixer counts is still there"
        );

        // A file with a picture keeps every one of them.
        let video = with_settings(ExportFormat::Video(lumit_media::encode::VideoCodec::H264));
        assert_eq!(video.render_options(), picture_settings);
        let patched = apply_render_overrides(&seeded, &video.render_options())
            .expect("a picture export still honours what was asked of it");
        assert!(!patched.comp(comp_id).unwrap().layers[0].switches.solo);
    }

    /// Every sample rate, sample width and channel layout the dialog offers,
    /// end to end through `run` and probed back off disk. The engine's answer
    /// and the file's own header have to be the same answer.
    #[test]
    fn the_audio_options_reach_the_written_file() {
        let (doc, comp) = solid_doc(32, 16);
        let dir = tempfile::tempdir().unwrap();
        for (format, ext) in [(AudioFormat::Wav, "wav"), (AudioFormat::M4a, "m4a")] {
            for rate in EXPORT_AUDIO_RATES.iter().copied() {
                for depth in AudioDepth::ALL {
                    for layout in AudioLayout::ALL {
                        let mut sp = spec(ExportFormat::Audio(format), 32, 16);
                        sp.include_audio = true;
                        sp.range = Some((0, 30)); // one second of a 30 fps comp
                        sp.audio_rate = rate;
                        sp.audio_depth = depth;
                        sp.audio_layout = layout;
                        // AAC has no sample width, so only the default stands
                        // there — that refusal is its own test below.
                        if sp.check().is_err() {
                            continue;
                        }
                        let path = dir.path().join(format!(
                            "mix-{ext}-{rate}-{}-{}.{ext}",
                            depth.bits(),
                            layout.channels()
                        ));
                        let (tx, _rx) = channel();
                        let cancel = AtomicBool::new(false);
                        run(&doc, comp, &[], &path, &sp, &tx, &cancel)
                            .unwrap_or_else(|e| panic!("{ext} {rate} Hz: {e}"));

                        let probe = lumit_media::probe::probe(&path).unwrap();
                        let audio = probe.audio.expect("it is all sound");
                        assert_eq!(
                            (audio.sample_rate as u32, audio.channels as u16),
                            (rate, layout.channels()),
                            "{ext} at {rate} Hz, {:?}",
                            layout
                        );
                        if format == AudioFormat::Wav {
                            let want = match depth {
                                AudioDepth::Sixteen => "pcm_s16le",
                                AudioDepth::TwentyFour => "pcm_s24le",
                            };
                            assert_eq!(audio.codec, want);
                        }
                        assert!(
                            (probe.duration_seconds - 1.0).abs() < 0.05,
                            "one second, not {}",
                            probe.duration_seconds
                        );
                    }
                }
            }
        }
    }

    /// A crop really crops: the same comp exported with and without one
    /// differs by exactly the pixels the crop took off, and the still's own
    /// size is the assertion.
    #[test]
    fn a_crop_decides_the_exported_frame_size() {
        let (doc, comp) = solid_doc(64, 32);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cropped.png");
        let mut sp = spec(
            ExportFormat::Images(lumit_media::encode::ImageFormat::Png),
            64,
            32,
        );
        sp.range = Some((0, 1));
        sp.crop = Crop {
            top: 4,
            left: 8,
            bottom: 4,
            right: 8,
        };
        let Some(result) = run_now(&doc, comp, &path, &sp) else {
            return;
        };
        result.expect("export runs");
        let frame = lumit_media::encode::sequence_frame_path(&path, 1);
        let probe = lumit_media::probe::probe(&frame).unwrap();
        let video = probe.video.expect("a still is a one-frame video");
        assert_eq!(
            (video.width, video.height),
            (48, 24),
            "64−8−8 by 32−4−4, in composition pixels"
        );
    }

    /// A sixteen-bit, alpha-carrying still export runs the whole way through
    /// — the pack stage, the wide encoder, and a file our own probe reads.
    #[test]
    fn a_sixteen_bit_still_export_runs_end_to_end() {
        let (doc, comp) = solid_doc(32, 16);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wide.png");
        let mut sp = spec(
            ExportFormat::Images(lumit_media::encode::ImageFormat::Png),
            32,
            16,
        );
        sp.range = Some((0, 2));
        sp.depth = BitDepth::Sixteen;
        sp.channels = Channels::RgbAlpha;
        sp.alpha = AlphaMode::Straight;
        let Some(result) = run_now(&doc, comp, &path, &sp) else {
            return;
        };
        result.expect("export runs");
        for n in 1..=2 {
            let frame = lumit_media::encode::sequence_frame_path(&path, n);
            let probe = lumit_media::probe::probe(&frame).unwrap();
            let video = probe.video.expect("frame {n} reads back");
            assert_eq!((video.width, video.height), (32, 16));
        }
    }

    /// A smooth float gradient exported at sixteen bits carries **more than
    /// 256 distinct values a channel** — the assertion that fails on the old
    /// widened path, where every value was a multiple of 257 and there were
    /// never more than 256 of them (the recorded ceiling).
    ///
    /// These are the two calls `run`'s frame loop makes at that depth, in that
    /// order, so the bytes counted here are the bytes the file gets; the file
    /// itself is proven by `a_sixteen_bit_still_export_runs_end_to_end`.
    #[test]
    fn a_sixteen_bit_export_carries_more_than_eight_bits_of_a_gradient() {
        let (doc, comp_id) = gradient_doc(512, 512);
        let mut renderer = match crate::headless::HeadlessRenderer::shared() {
            Ok(r) => r,
            Err(_) => {
                lumit_gpu::no_adapter();
                return;
            }
        };
        let quality = crate::plan::Quality::default();
        let (deep, w, h) = renderer
            .render_preview16(&doc, comp_id, 0, quality)
            .expect("the deep read-back runs");
        assert_eq!((w, h), (512, 512));
        let bytes = pack_frame(&deep, Channels::RgbAlpha, AlphaMode::Premultiplied, None);
        assert_eq!(bytes.len(), deep.len() * 2, "two bytes a channel");
        let reds: Vec<u16> = bytes
            .chunks_exact(8)
            .map(|px| u16::from_le_bytes([px[0], px[1]]))
            .collect();
        let distinct: std::collections::BTreeSet<u16> = reds.iter().copied().collect();
        assert!(
            distinct.len() > 256,
            "a sixteen-bit gradient must hold more than eight bits' worth: {} values",
            distinct.len()
        );
        assert!(
            reds.iter().any(|v| v % 257 != 0),
            "values no widened byte could produce"
        );

        // The same frame at eight bits cannot, by construction — which is why
        // widening it was a ceiling rather than an implementation.
        let (shallow, _, _) = renderer
            .render_preview(&doc, comp_id, 0, quality, 1.0)
            .expect("the eight-bit read-back runs");
        let shallow: std::collections::BTreeSet<u8> =
            shallow.chunks_exact(4).map(|px| px[0]).collect();
        assert!(shallow.len() <= 256);
    }

    /// The file says what it is. An mp4 carries a `colr` box in `nclx` form —
    /// three ISO/IEC 23091-2 code points, sixteen bits each — and a player
    /// that cannot read it has to guess, which is how a wide-gamut delivery
    /// comes back looking wrong. Exported twice, the bytes are identical, so
    /// the colour transform costs the export nothing in determinism.
    #[test]
    fn the_colour_space_reaches_the_containers_colr_box() {
        /// The `(primaries, transfer, matrix)` of the file's `colr` box.
        fn nclx(bytes: &[u8]) -> Option<(u16, u16, u16)> {
            let at = bytes
                .windows(8)
                .position(|w| &w[..4] == b"colr" && &w[4..] == b"nclx")?;
            let p = at + 8;
            let be = |i: usize| u16::from_be_bytes([bytes[i], bytes[i + 1]]);
            (bytes.len() > p + 6).then(|| (be(p), be(p + 2), be(p + 4)))
        }

        let (doc, comp) = solid_doc(32, 16);
        let dir = tempfile::tempdir().unwrap();

        // The default space: sRGB — Rec.709 primaries (1), the IEC 61966-2-1
        // curve (13), a Rec.709 matrix (1).
        let plain = dir.path().join("srgb.mp4");
        let mut sp = spec(ExportFormat::Video(VideoCodec::H264), 32, 16);
        sp.range = Some((0, 5));
        let Some(result) = run_now(&doc, comp, &plain, &sp) else {
            return;
        };
        result.expect("export runs");
        let bytes = std::fs::read(&plain).unwrap();
        assert_eq!(
            nclx(&bytes),
            Some((1, 13, 1)),
            "an untagged-looking export still states sRGB"
        );

        // Rec.2020: primaries 9, the BT.2020 ten-bit curve 14, the
        // non-constant-luminance matrix 9.
        let wide = dir.path().join("rec2020.mp4");
        sp.colour_space = ColourSpace::Rec2020;
        run_now(&doc, comp, &wide, &sp)
            .expect("the pipeline was there a moment ago")
            .expect("export runs");
        let wide_bytes = std::fs::read(&wide).unwrap();
        assert_eq!(nclx(&wide_bytes), Some((9, 14, 9)), "Rec.2020 is stated");

        // The pixels changed too, not just the label — a file that says 2020
        // and carries 709 numbers is exactly the lie the tag exists to stop.
        assert_ne!(bytes, wide_bytes, "the transform reached the picture");

        // And it is deterministic: the same spec writes the same file.
        let again = dir.path().join("rec2020-again.mp4");
        run_now(&doc, comp, &again, &sp)
            .expect("the pipeline was there a moment ago")
            .expect("export runs");
        assert_eq!(
            wide_bytes,
            std::fs::read(&again).unwrap(),
            "two runs of one spec write the same bytes"
        );
    }

    /// One built-in audio effect on a rack of its own, with `over` written
    /// into its rows.
    ///
    /// A chain with no wires in it, so the document and the two ids are never
    /// read: `bake_values` only builds an expression context for a parameter a
    /// wire feeds.
    fn audio_rack(match_name: &str, over: &[(&str, f64)]) -> Arc<AudioChain> {
        rack_of(vec![audio_effect(match_name, over)])
    }

    /// One built-in audio effect, with `over` written into its rows.
    fn audio_effect(match_name: &str, over: &[(&str, f64)]) -> lumit_core::model::EffectInstance {
        use lumit_core::anim::Property;
        use lumit_core::model::EffectValue;

        let mut instance = lumit_core::fx::instantiate(match_name).expect("a catalogue entry");
        for param in &mut instance.params {
            if let Some((_, value)) = over.iter().find(|(id, _)| *id == param.id) {
                param.value = EffectValue::Float(Property::fixed(*value));
            }
        }
        instance
    }

    /// A rack of them.
    fn rack_of(effects: Vec<lumit_core::model::EffectInstance>) -> Arc<AudioChain> {
        Arc::new(AudioChain {
            doc: Arc::new(Document::new()),
            comp: Uuid::nil(),
            layer: Uuid::nil(),
            effects,
            graph: lumit_core::graph::LayerGraph::default(),
            offset_s: 0.0,
            base_s: 0.0,
        })
    }

    /// A job carrying the two racks and nothing else. Both are `None` for a
    /// layer whose switches are off, which is what
    /// `a_clip_has_its_own_chain_and_each_fx_switch_drops_one` in
    /// `crate::headless` pins the switches to.
    fn racked_job(clip: Option<Arc<AudioChain>>, row: Option<Arc<AudioChain>>) -> AudioJob {
        AudioJob {
            item: Uuid::nil(),
            layer: Uuid::nil(),
            clip: None,
            path: PathBuf::new(),
            in_s: 0.0,
            out_s: 1.0,
            offset_s: 0.0,
            volume: lumit_core::anim::Property::zero(),
            pan: lumit_core::anim::Property::zero(),
            carriers: Vec::new(),
            fade: None,
            driven: None,
            chain: row,
            clip_chain: clip,
        }
    }

    /// The loudest sample in a run, either channel.
    fn loudest(samples: &[f32]) -> f32 {
        samples.iter().fold(0.0f32, |top, s| top.max(s.abs()))
    }

    /// **The clip's rack runs ahead of the row's, and a rack that is gone is
    /// simply not run** (docs/impl/audio-effects.md §6 plan 4,
    /// docs/impl/audio-timeline.md §4).
    ///
    /// A gain of +6 dB on the clip and a limiter on the row. The gain drives
    /// the sound over the ceiling and the limiter, hearing it afterwards,
    /// holds it there; the other order would let the +6 straight out. So the
    /// peak alone says which way round the two racks ran.
    ///
    /// Each rack arrives as `None` when its switch is off: the clip's `fx`
    /// and the layer's `switches.fx`, both pinned where the jobs are built.
    /// This is the other half of that, because a chain that is gone changes
    /// the sound back.
    #[test]
    fn a_clips_rack_runs_ahead_of_the_rows_and_a_missing_rack_is_not_run() {
        let rate = 48_000u32;
        let frames = rate as usize / 4;
        // A quarter second at half scale, which is a whisker under the −6 dB
        // ceiling the limiter is set to below.
        let input: Vec<f32> = (0..frames)
            .flat_map(|n| {
                let phase = std::f64::consts::TAU * 220.0 * n as f64 / f64::from(rate);
                [phase.sin() as f32 * 0.5; 2]
            })
            .collect();
        let ceiling = 10f32.powf(-6.0 / 20.0);
        let gain = || Some(audio_rack("audio_gain", &[("gain", 6.0)]));
        let limiter = || Some(audio_rack("audio_limiter", &[("ceiling", -6.0)]));

        let (both, _) = job_bake(&racked_job(gain(), limiter()), &input, 0, rate, true)
            .expect("two racks, two effects");
        assert!(
            loudest(&both) < ceiling * 1.05,
            "the limiter heard the +6 dB: peak {}",
            loudest(&both)
        );

        // The clip's rack alone: the +6 dB comes out untouched, which is what
        // the other order would have produced above.
        let (clip_only, _) =
            job_bake(&racked_job(gain(), None), &input, 0, rate, true).expect("the clip's rack");
        assert!(
            loudest(&clip_only) > ceiling * 1.8,
            "nothing held the gain back: peak {}",
            loudest(&clip_only)
        );

        // The row's rack alone: the sound was already under the ceiling, so
        // the limiter leaves it where it was.
        let (row_only, _) =
            job_bake(&racked_job(None, limiter()), &input, 0, rate, true).expect("the row's rack");
        assert!(loudest(&row_only) < ceiling * 1.05);

        // And neither rack is the mix untouched, byte for byte.
        assert!(
            job_bake(&racked_job(None, None), &input, 0, rate, true).is_none(),
            "no rack, no bake"
        );
    }

    /// **A Precomp layer's rack hears the sum, and the layer's Volume rides
    /// on what comes back** (docs/09 §3.1, the bus stage).
    ///
    /// Two copies of the same half-scale tone arrive through one Precomp
    /// layer whose rack is a limiter set to −6 dB. Either one alone is already
    /// under that ceiling, so only the sum can move the limiter: if the rack
    /// were run per source the two would come out untouched and add to full
    /// scale. The peak alone therefore says whether the sum existed.
    ///
    /// Then the same mix with the Precomp layer at −6 dB. The fader is
    /// **after** the insert, so it halves the held sum; riding inside the rack
    /// it would have taken each source under the ceiling and the limiter would
    /// never have bitten.
    #[test]
    fn a_precomp_layers_rack_hears_the_summed_comp() {
        let rate = 48_000u32;
        let frames = rate as usize / 4;
        let ceiling = 10f32.powf(-6.0 / 20.0);
        let tone = Arc::new(lumit_media::AudioBuffer {
            rate,
            samples: (0..frames)
                .flat_map(|n| {
                    let phase = std::f64::consts::TAU * 220.0 * n as f64 / f64::from(rate);
                    [phase.sin() as f32 * 0.5; 2]
                })
                .collect(),
        });
        let bus = |volume_db: f64| Carrier {
            volume: lumit_core::anim::Property::fixed(volume_db),
            pan: lumit_core::anim::Property::zero(),
            offset_s: 0.0,
            fade: None,
            chain: Some(audio_rack("audio_limiter", &[("ceiling", -6.0)])),
        };
        let row = Uuid::now_v7();
        let through = |carrier: Carrier| {
            let one = |item: Uuid| {
                let mut job = racked_job(None, None);
                job.item = item;
                job.layer = row;
                job.out_s = 0.25;
                job.carriers = vec![carrier.clone()];
                job
            };
            let decoded = [
                (Arc::clone(&tone), one(Uuid::now_v7())),
                (Arc::clone(&tone), one(Uuid::now_v7())),
            ];
            mixdown_prepared(&decoded, rate, 0.25, 1.0)
        };

        let held = through(bus(0.0));
        assert!(
            loudest(&held) < ceiling * 1.05 && loudest(&held) > ceiling * 0.5,
            "the limiter heard both sources at once: peak {}",
            loudest(&held)
        );

        let quieter = through(bus(-6.0));
        assert!(
            (loudest(&quieter) - loudest(&held) * ceiling).abs() < 0.02,
            "the Precomp layer's Volume rides on what the rack gave back: \
             peak {} against {}",
            loudest(&quieter),
            loudest(&held)
        );

        // And with no rack on the carrier nothing is summed early: the two
        // sources reach full scale and the master's own ceiling holds them.
        let mut plain = bus(0.0);
        plain.chain = None;
        let open = through(plain);
        assert!(
            loudest(&open) > ceiling * 1.5,
            "a carrier with no rack places its sources as it always did: \
             peak {}",
            loudest(&open)
        );
    }

    /// **A click through the distortion lands at the same comp time as
    /// without it** (plan 2).
    ///
    /// The oversampler's half-band pair is linear phase, so the whole click
    /// arrives late by one fixed number of frames. The bake reports it and the
    /// mixer places the run that many frames earlier, which puts the click
    /// back where it was.
    #[test]
    fn a_click_through_the_distortion_lands_at_the_same_comp_time() {
        let rate = 48_000u32;
        let frames = 4_096usize;
        let at = 1_000usize;
        let mut input = vec![0.0f32; frames * 2];
        input[at * 2] = 0.2;
        input[at * 2 + 1] = 0.2;

        let job = racked_job(
            None,
            Some(audio_rack("audio_distortion", &[("drive", 0.0)])),
        );
        let (wet, latency) = job_bake(&job, &input, 0, rate, true).expect("the distortion opens");
        assert!(latency > 0, "the half-band pair's delay is reported");
        let peak = (0..wet.len() / 2)
            .max_by(|a, b| wet[a * 2].abs().total_cmp(&wet[b * 2].abs()))
            .expect("a frame to be loudest");
        assert_eq!(
            peak as i64 - i64::from(latency),
            at as i64,
            "placed {latency} frames earlier the click is back where it started"
        );
    }
}

//! What one sound in the mix is: its job, its retime, its racks, its clip
//! fades and the Precomp layers it arrives through.

use std::sync::Arc;
use uuid::Uuid;

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
    /// The file the sound is read from, and which of its audio streams: the
    /// first for nearly every layer, another or several mixed for a layer or
    /// clip that has chosen (`lumit_core::model::AudioStreams`).
    pub sound: lumit_media::audio::SoundSource,
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
    /// The retime the sound plays through, for a layer or clip that isn't at
    /// its source's own speed. `None` plays the file as it is.
    pub retime: Option<Arc<Retimed>>,
}

/// A retime the sound follows: at comp time `t` it plays source second
/// `map(t - origin_s)`.
#[derive(Clone, PartialEq)]
pub struct Retimed {
    pub map: lumit_core::anim::Property,
    pub origin_s: f64,
    /// The source seconds a clip is trimmed to. Silent outside them.
    pub held: Option<(f64, f64)>,
    /// Pitch rises and falls with the speed, as on tape. Off keeps it.
    pub pitch_follows_speed: bool,
}

/// A wire onto a layer's Volume (the Audio workspace board's *Duck under*):
/// everything [`super::bake::volume_bake`] needs to ask the driver chain for the decibels
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
        lumit_core::fx::drivers::driven_volume_db(
            &self.graph,
            t - self.offset_s,
            context,
            Some(&tap),
        )
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
/// when the chain opens ([`lumit_core::fx::registry::EffectDef::open_audio`]). A stack
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
    /// [`super::bake::volume_bake`] reads a constant off `gain_at(0.0)` without needing an
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
    /// The Precomp layer's or composition clip's retime. Everything arriving
    /// through this carrier is summed and the sum read through the map, ahead
    /// of the rack. `None` on a carrier that plays at its own speed.
    pub retime: Option<std::sync::Arc<BusRetime>>,
}

/// A retime on a whole comp's sound: what arrives through a [`Carrier`] is
/// summed, and the sum is read through `retime` over `span`.
#[derive(Clone, PartialEq)]
pub struct BusRetime {
    pub retime: Retimed,
    /// Where the comp's own time 0 sits on the mixed timeline.
    pub zero_s: f64,
    /// The layer's or clip's span on the mixed timeline.
    pub span: (f64, f64),
}

/// The source seconds `retimed` reaches over `span` of the mixed timeline,
/// with a tenth of a second either side. `None` where it reaches nothing.
#[must_use]
pub fn retimed_reach(retimed: &Retimed, span: (f64, f64)) -> Option<(f64, f64)> {
    let steps = ((span.1 - span.0) * 100.0).ceil().max(1.0) as usize;
    let (lo, hi) = (0..=steps)
        .map(|i| {
            let t = span.0 + (span.1 - span.0) * i as f64 / steps as f64;
            retimed.map.value_at(t - retimed.origin_s)
        })
        .filter(|s| s.is_finite())
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), s| {
            (a.min(s), b.max(s))
        });
    (lo <= hi).then_some((lo - 0.1, hi + 0.1))
}

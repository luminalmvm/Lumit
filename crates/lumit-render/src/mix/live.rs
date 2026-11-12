//! The live plan playback reads, and the rack bakes it keeps between builds.

use lumit_audio::mix::MixPlan;
use lumit_audio::stream::Source;
use std::sync::Arc;
use uuid::Uuid;

use super::bake::{job_bake, volume_bake};
use super::jobs::{AudioChain, AudioJob, Retimed};
use super::mixdown::{buses, dry_run, place};
use super::stage::{in_a_bus, RunOf};

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
    /// Which of the item's audio streams went through the racks: two rows
    /// split from one recording share an item, a span and a rack, and
    /// differ in this alone.
    streams: Vec<u32>,
    rate: u32,
    start_frame: i64,
    src_start: usize,
    len: usize,
    clip_chain: Option<Arc<AudioChain>>,
    chain: Option<Arc<AudioChain>>,
    retime: Option<Arc<Retimed>>,
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
/// ([`super::mixdown::mixdown_counting`]), in the same order, so playback sounds like the
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
                        source: Source::whole(Arc::new(lumit_media::audio::AudioBuffer {
                            rate,
                            samples,
                        })),
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
        // A retimed job is worked out ahead like a racked one, since the
        // file can't be read straight off at its speed.
        let racked = job.chain.is_some() || job.clip_chain.is_some() || job.retime.is_some();
        let placed = if job.retime.is_some() {
            Some(((job.in_s * f64::from(rate)).round() as i64, 0, 0))
        } else {
            place(source, job, rate, racked)
        };
        let Some((start_frame, src_start, len)) = placed else {
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
                streams: job.sound.streams().to_vec(),
                rate,
                start_frame,
                src_start,
                len,
                clip_chain: job.clip_chain.clone(),
                chain: job.chain.clone(),
                retime: job.retime.clone(),
            };
            let baked = match bakes.held.iter().position(|(k, _)| *k == key) {
                Some(at) => bakes.held.swap_remove(at).1,
                None => dry_run(source, job, rate, false).and_then(|(at, dry)| {
                    let (samples, latency) = job_bake(job, &dry, at, rate, false)
                        .or_else(|| job.retime.is_some().then_some((dry, 0)))?;
                    Some((
                        Source::whole(Arc::new(lumit_media::audio::AudioBuffer { rate, samples })),
                        latency,
                    ))
                }),
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

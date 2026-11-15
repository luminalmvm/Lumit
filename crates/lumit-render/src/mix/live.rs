//! The jobs placed as clips on their nodes, and the rack bakes kept between
//! builds.

use lumit_audio::plan::PlacedClip;
use lumit_audio::stream::Source;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use uuid::Uuid;

use super::bake::{job_bake, volume_bake};
use super::graph::Cancelled;
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
/// build on all three is handed the same sound back. A node's rack that has
/// to hear its whole input is kept the same way, by what it heard.
///
/// Holds what the last plan held and no more: a build takes what it can use
/// and leaves behind only what it used. A build that is stopped leaves it as
/// it was.
#[derive(Default)]
pub struct RackBakes {
    held: Vec<(BakeKey, Baked)>,
    pub(super) whole: Vec<(WholeKey, Baked)>,
    /// Whatever else the racks' answers depend on that the keys cannot see:
    /// the caller's own stamp, and a changed one empties the lot.
    pub(super) stamp: u64,
}

/// What a job's racks came to: the processed sound and the latency it is
/// placed early by, or `None` where neither rack opened anything as audio.
pub(super) type Baked = Option<(Arc<Source>, u32)>;

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

/// What a whole-input node rack heard: its chain, where its input starts and
/// the input's own fingerprint.
#[derive(PartialEq)]
pub(super) struct WholeKey {
    pub chain: Arc<AudioChain>,
    pub rate: u32,
    pub start_frame: i64,
    pub heard: [u8; 32],
}

impl RackBakes {
    /// Forget every bake unless `stamp` is the one they were made under. The
    /// caller folds in what a rack's sound depends on beyond the job itself,
    /// which today is the list of plugins switched off for now.
    pub fn stamped(&mut self, stamp: u64) {
        if self.stamp != stamp {
            self.held.clear();
            self.whole.clear();
            self.stamp = stamp;
        }
    }

    /// How many bytes of processed sound the bakes hold.
    #[must_use]
    pub fn bytes(&self) -> usize {
        self.held
            .iter()
            .map(|(_, baked)| baked)
            .chain(self.whole.iter().map(|(_, baked)| baked))
            .filter_map(Option::as_ref)
            .map(|(source, _)| source.frames().unwrap_or(0) * 2 * std::mem::size_of::<f32>())
            .sum()
    }
}

/// Place the jobs as clips, each on the node `node_of` names for its strip
/// ([`AudioJob::layer`]).
///
/// The same placement, racks, bus stage and gain bake whether the plan is
/// played or exported. A job with no rack on it reads nothing here: it names
/// its source and the stretch it plays, and the blocks are decoded as the
/// mix nears them. Offline every file's exact length is found and racks run
/// their slower path. A job whose source `source_of` does not know, or knows
/// at another rate, contributes nothing.
///
/// `cancel` stops it between jobs and between rack blocks; the bakes are then
/// left as they were.
pub(super) fn place_clips(
    jobs: &[AudioJob],
    source_of: &dyn Fn(&AudioJob) -> Option<Arc<Source>>,
    node_of: &dyn Fn(Uuid) -> u16,
    rate: u32,
    offline: bool,
    bakes: &mut RackBakes,
    cancel: &AtomicBool,
) -> Result<Vec<PlacedClip>, Cancelled> {
    let stopped = || cancel.load(Ordering::Relaxed);
    let sourced: Vec<(Arc<Source>, &AudioJob)> = jobs
        .iter()
        .filter_map(|job| Some((source_of(job).filter(|s| s.rate() == rate)?, job)))
        .collect();
    let mut kept: Vec<(BakeKey, Baked)> = Vec::new();
    // Stopped part way: what was taken goes back, and nothing new is kept.
    let give_back = |bakes: &mut RackBakes, kept: Vec<(BakeKey, Baked)>| {
        bakes.held.extend(kept);
        Err(Cancelled)
    };
    let mut clips = Vec::with_capacity(sourced.len());
    let staged = buses(&sourced, rate, offline, cancel);
    if stopped() {
        return Err(Cancelled);
    }
    let mut staged = staged.into_iter().peekable();
    for (index, (source, job)) in sourced.iter().enumerate() {
        if stopped() {
            return give_back(bakes, kept);
        }
        if in_a_bus(job) {
            // The bus stage: everything arriving through a nested comp's
            // rack or retime was summed and run through it, and comes back as
            // one run of its own on the strip it arrives on (docs/09 §3.1).
            if let Some((_, run)) = staged.next_if(|(at, _)| *at == index) {
                if let RunOf::Bus(samples) = run.of {
                    let frames = samples.len() / 2;
                    clips.push(PlacedClip {
                        source: Source::whole(Arc::new(lumit_media::audio::AudioBuffer {
                            rate,
                            samples,
                        })),
                        start_frame: run.start_frame,
                        src_start: 0,
                        len: frames,
                        gain: run.gain,
                        envelope: run.envelope.map(Arc::new),
                        node: node_of(run.layer),
                        silent: false,
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
            place(source, job, rate, racked || offline)
        };
        let Some((start_frame, src_start, len)) = placed else {
            continue;
        };
        // The clip's insert chain (and a strip's own where it has no node of
        // its own), ahead of the gains. The processed span **replaces** the
        // file in the plan, so the mix plays finished sound and never waits
        // on a plugin; a job whose stacks open nothing keeps its file and
        // reads it in blocks like any other.
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
                None => dry_run(source, job, rate, offline).and_then(|(at, dry)| {
                    let (samples, latency) = job_bake(job, &dry, at, rate, offline, cancel)
                        .or_else(|| job.retime.is_some().then_some((dry, 0)))?;
                    Some((
                        Source::whole(Arc::new(lumit_media::audio::AudioBuffer { rate, samples })),
                        latency,
                    ))
                }),
            };
            if stopped() {
                return give_back(bakes, kept);
            }
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
        clips.push(PlacedClip {
            source,
            start_frame,
            src_start,
            len,
            gain,
            envelope: envelope.map(Arc::new),
            node: node_of(job.layer),
            silent: false,
        });
    }
    bakes.held = kept;
    Ok(clips)
}

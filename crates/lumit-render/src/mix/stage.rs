//! The bus stage: sound arriving through a Precomp layer with a rack or a
//! retime is summed and run through it before it reaches the mix.

use uuid::Uuid;

use std::sync::atomic::AtomicBool;

use super::bake::{chain_bake_until, gain_bake};
use super::jobs::{AudioJob, Carrier};
use super::mixdown::read_retimed;

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
/// gain [`super::bake::volume_bake`] bakes, so a mix with no rack in it is the mix it was.
///
/// `cancel` stops the racks between blocks; what comes back then is not to be
/// kept.
#[must_use]
pub fn bus_runs(
    placed: &[PlacedJob<'_>],
    rate: u32,
    offline: bool,
    cancel: &AtomicBool,
) -> Vec<MixRun> {
    let all: Vec<usize> = (0..placed.len()).collect();
    runs_at(placed, &all, 0, rate, offline, cancel)
}

/// The first carrier at or past `depth` that holds a rack or a retime: where
/// this job's sound is summed on its way out.
fn bus_at(job: &AudioJob, depth: usize) -> Option<usize> {
    job.carriers
        .iter()
        .enumerate()
        .skip(depth)
        .find_map(|(at, c)| bus_mark(c).is_some().then_some(at))
}

/// What makes a carrier a bus, as one pointer: its rack, or else its retime.
/// Two carriers with the same mark are the same bus.
fn bus_mark(c: &Carrier) -> Option<*const ()> {
    c.chain
        .as_ref()
        .map(|chain| std::sync::Arc::as_ptr(chain).cast::<()>())
        .or_else(|| {
            c.retime
                .as_ref()
                .map(|r| std::sync::Arc::as_ptr(r).cast::<()>())
        })
}

/// The runs inside one bus: `which` indexes [`bus_runs`]'s input and `depth`
/// is the first carrier whose gain has not been spent yet.
fn runs_at(
    placed: &[PlacedJob<'_>],
    which: &[usize],
    depth: usize,
    rate: u32,
    offline: bool,
    cancel: &AtomicBool,
) -> Vec<MixRun> {
    let mut out: Vec<MixRun> = Vec::new();
    // The buses already summed, so the rest of a bus's jobs are passed over
    // rather than summed again. Two Precomp layers at the same depth are two
    // buses, which is why the chain itself is the mark and not the depth.
    let mut summed: Vec<(usize, *const ())> = Vec::new();
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
        let carrier = &job.carriers[at];
        let Some(mark) = bus_mark(carrier) else {
            continue;
        };
        if summed.contains(&(at, mark)) {
            continue;
        }
        summed.push((at, mark));
        let group: Vec<usize> = which
            .iter()
            .copied()
            .filter(|&j| {
                bus_at(placed[j].job, depth) == Some(at)
                    && bus_mark(&placed[j].job.carriers[at]) == Some(mark)
            })
            .collect();
        let inner = runs_at(placed, &group, at + 1, rate, offline, cancel);
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
        // A retimed comp's sum is read through the map first, so the rack
        // hears it at its new speed, as a footage layer's rack does.
        let (sum, first) = match &carrier.retime {
            Some(bus) => {
                let zero = bus.zero_s * f64::from(rate) - first as f64;
                let frames = sum.len() / 2;
                let Some((at, out)) = read_retimed(
                    &bus.retime,
                    bus.span,
                    zero,
                    rate,
                    || frames,
                    |a, n| sum[a * 2..(a + n) * 2].to_vec(),
                ) else {
                    continue;
                };
                (out, at)
            }
            None => (sum, first),
        };
        let baked = carrier
            .chain
            .as_ref()
            .and_then(|chain| chain_bake_until(chain, &sum, first, rate, offline, cancel));
        let (samples, start_frame) = match baked {
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

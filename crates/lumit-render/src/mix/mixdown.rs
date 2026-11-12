//! The mixdown an export writes, and the reads and placement the live plan
//! shares with it.

use lumit_audio::stream::Source;
use std::sync::Arc;

use super::bake::{job_bake, volume_bake};
use super::jobs::{AudioJob, Retimed};
use super::stage::{bus_runs, in_a_bus, MixRun, PlacedJob, RunOf};

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
        .map(|job| (pool.source(&job.sound, rate), job))
        .collect();
    mix_sources(&sourced, rate, duration_s, master_gain, on_decoded)
}

/// As [`mixdown`], but over buffers already in memory.
pub fn mixdown_prepared(
    decoded: &[(std::sync::Arc<lumit_media::audio::AudioBuffer>, AudioJob)],
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
pub(super) fn place(
    source: &Source,
    job: &AudioJob,
    rate: u32,
    wait: bool,
) -> Option<(i64, usize, usize)> {
    let frames = if wait {
        source.frames_exactly()
    } else {
        // Long enough that no clip is cut short by it, short enough that
        // adding to it cannot overflow.
        source.frames().unwrap_or(usize::MAX / 4)
    };
    lumit_audio::mix::place_on_timeline(job.in_s, job.out_s, job.offset_s, frames, rate)
}

/// What a job plays before its racks and its Volume: where it starts on the
/// timeline and the samples. A retimed job is read through its map, the rest
/// straight from the file.
pub(super) fn dry_run(
    source: &Arc<Source>,
    job: &AudioJob,
    rate: u32,
    exact: bool,
) -> Option<(i64, Vec<f32>)> {
    let Some(retimed) = &job.retime else {
        let (start_frame, src_start, len) = place(source, job, rate, true)?;
        return Some((start_frame, source.read(src_start, len, exact)));
    };
    read_retimed(
        retimed,
        (job.in_s, job.out_s),
        0.0,
        rate,
        || source.frames_exactly(),
        |first, len| source.read(first, len, exact),
    )
}

/// Sound read through `retimed` over `span` of the mixed timeline: where it
/// starts and the samples. Source second `s` is frame `s * rate + zero` of
/// what `read` reads, and `total` says how many frames that runs to.
pub(super) fn read_retimed(
    retimed: &Retimed,
    span: (f64, f64),
    zero: f64,
    rate: u32,
    total: impl FnOnce() -> usize,
    read: impl FnOnce(usize, usize) -> Vec<f32>,
) -> Option<(i64, Vec<f32>)> {
    let r = f64::from(rate);
    let start_frame = (span.0 * r).round() as i64;
    let frames = usize::try_from((span.1 * r).round() as i64 - start_frame)
        .ok()
        .filter(|f| *f > 0)?;
    // Where each output frame reads the source, one entry every STEP frames.
    let step = lumit_audio::speed::STEP;
    let mut table: Vec<f64> = (0..=frames / step + 1)
        .map(|c| {
            let t = (start_frame as f64 + (c * step) as f64) / r;
            let s = retimed.map.value_at(t - retimed.origin_s);
            match retimed.held {
                Some((lo, hi)) if s < lo || s > hi => f64::NAN,
                _ => s * r + zero,
            }
        })
        .collect();
    let (lo, hi) = table
        .iter()
        .filter(|p| p.is_finite())
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| {
            (a.min(*p), b.max(*p))
        });
    // Only the stretch of source the map reaches, with room for the grains.
    let pad = r / 10.0;
    let first = (lo - pad).floor().max(0.0);
    let first = if first.is_finite() { first as usize } else { 0 };
    let last = if hi.is_finite() {
        ((hi + pad).ceil().max(0.0) as usize).min(total())
    } else {
        0
    };
    let src = if last > first {
        read(first, last - first)
    } else {
        Vec::new()
    };
    for p in &mut table {
        *p -= first as f64;
    }
    let samples = if retimed.pitch_follows_speed {
        lumit_audio::speed::tape(&src, &table, frames)
    } else {
        lumit_audio::speed::keep_pitch(&src, &table, frames, rate)
    };
    Some((start_frame, samples))
}

/// **The buses of a mix**, each with the index of the job it stands in line
/// at: every job arriving through a Precomp layer that carries a rack, read,
/// run through its own racks, summed and run through the layer's
/// ([`bus_runs`]). These are the jobs that have to be in memory together,
/// because a rack hears a sum.
pub(super) fn buses(
    sourced: &[(Arc<Source>, &AudioJob)],
    rate: u32,
    offline: bool,
) -> Vec<(usize, MixRun)> {
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
            let (start_frame, dry) = dry_run(source, job, rate, offline)?;
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
        } else if let Some((start_frame, dry)) = dry_run(source, job, rate, true) {
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

/// How many audio samples (per channel) belong before the end of video
/// frame `frame_count` — the A/V interleaving rule. Cumulative rounding, so
/// the running total never drifts from `frames / fps × rate`.
pub fn audio_samples_through(frame_count: usize, fps: f64, rate: u32) -> usize {
    if fps <= 0.0 {
        return 0;
    }
    ((frame_count as f64 / fps) * f64::from(rate)).round() as usize
}

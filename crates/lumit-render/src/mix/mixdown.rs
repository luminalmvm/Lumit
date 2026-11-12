//! The mixdown an export writes, and the reads and placement every plan
//! shares.

use lumit_audio::stream::Source;
use lumit_core::model::{Composition, Document};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use super::bake::job_bake;
use super::graph::build_graph;
use super::jobs::{AudioJob, Retimed};
use super::live::RackBakes;
use super::render::render;
use super::stage::{bus_runs, in_a_bus, MixRun, PlacedJob};

/// **The comp's mix**, as an export writes it: the jobs built into the comp's
/// desk ([`build_graph`], offline) and played from frame 0 with every rack
/// run in order ([`render`]), so preview, beat detection and export cannot
/// disagree about what the comp sounds like.
///
/// `desk` false leaves the master's fader and limiter out (a plain clamp at
/// the ceiling stays): the mix before the desk, for beat detection, whose
/// onsets are relative and must not be lost because somebody pulled the
/// master down. `listen` is a layer asked for by name, heard whatever its
/// switches and any solo say ([`build_graph`]). `on_progress` is handed how
/// far through the run is, 0 to 1.
///
/// Each file is read through the pool the mixer plays from
/// ([`lumit_audio::stream`]) as the run reaches it; what is held whole is the
/// mix and any racked clip's baked sound.
#[allow(clippy::too_many_arguments)]
pub fn mixdown(
    doc: &Arc<Document>,
    comp: &Composition,
    jobs: Vec<AudioJob>,
    rate: u32,
    duration_s: f64,
    desk: bool,
    listen: Option<uuid::Uuid>,
    on_progress: &mut dyn FnMut(f64),
) -> Vec<f32> {
    let pool = lumit_audio::stream::pool();
    mix_sources(
        doc,
        comp,
        jobs,
        &|job| Some(pool.source(&job.sound, rate)),
        rate,
        duration_s,
        desk,
        listen,
        on_progress,
    )
}

/// As [`mixdown`], through the desk, over buffers already in memory: each job
/// sounds from the buffer paired with a job of the same item and sound.
pub fn mixdown_prepared(
    doc: &Arc<Document>,
    comp: &Composition,
    decoded: &[(Arc<lumit_media::audio::AudioBuffer>, AudioJob)],
    rate: u32,
    duration_s: f64,
) -> Vec<f32> {
    let jobs = decoded.iter().map(|(_, j)| j.clone()).collect();
    let source_of = |job: &AudioJob| {
        decoded
            .iter()
            .find(|(_, j)| j.item == job.item && j.sound == job.sound)
            .map(|(b, _)| Source::whole(Arc::clone(b)))
    };
    mix_sources(
        doc,
        comp,
        jobs,
        &source_of,
        rate,
        duration_s,
        true,
        None,
        &mut |_| {},
    )
}

/// The desk built offline over `source_of`'s sources and played whole.
#[allow(clippy::too_many_arguments)]
fn mix_sources(
    doc: &Arc<Document>,
    comp: &Composition,
    jobs: Vec<AudioJob>,
    source_of: &dyn Fn(&AudioJob) -> Option<Arc<Source>>,
    rate: u32,
    duration_s: f64,
    desk: bool,
    listen: Option<uuid::Uuid>,
    on_progress: &mut dyn FnMut(f64),
) -> Vec<f32> {
    let never = AtomicBool::new(false);
    let built = build_graph(
        doc,
        comp,
        jobs,
        source_of,
        rate,
        duration_s,
        &mut RackBakes::default(),
        None,
        true,
        &never,
        listen,
    );
    let Ok((mut plan, _)) = built else {
        return Vec::new();
    };
    if !desk {
        plan.master = lumit_audio::plan::Fader::default();
    }
    render(Arc::new(plan), rate, desk, on_progress)
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
    cancel: &AtomicBool,
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
        .take_while(|_| !cancel.load(std::sync::atomic::Ordering::Relaxed))
        .filter_map(|(index, (source, job))| {
            let (start_frame, dry) = dry_run(source, job, rate, offline)?;
            Some(
                match job_bake(job, &dry, start_frame, rate, offline, cancel) {
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
                },
            )
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
    bus_runs(&staged, rate, offline, cancel)
        .into_iter()
        .map(|run| (members[run.first].index, run))
        .collect()
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

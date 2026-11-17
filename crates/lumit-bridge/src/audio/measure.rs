//! Measuring a comp's whole mix, and moving its master to a loudness.
//!
//! The mix is built and played offline exactly as an export plays it, racks
//! and all, through the master fader and the limiter at the comp's ceiling,
//! as playback hears it, and its loudness read as it goes. Runs on the
//! caller's thread for as long as that takes; one at a time, and stopped by
//! [`stop`].

use lumit_audio::plan::MixPlan;
use lumit_core::model::Document;
use lumit_render::mix::graph::build_graph;
use lumit_render::mix::render::{land, Measured};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use uuid::Uuid;

/// The rate a mix is measured at.
const RATE: u32 = 48_000;

static MEASURING: AtomicBool = AtomicBool::new(false);
static STOP: AtomicBool = AtomicBool::new(false);

/// Why a measure has no answer.
pub(crate) enum Refused {
    /// Another measure is running.
    Busy,
    /// Stopped before it finished.
    Stopped,
    /// Nothing in the mix is heard.
    Silent,
}

/// Stop the measure running now, if there is one.
pub(crate) fn stop() {
    STOP.store(true, Ordering::Relaxed);
}

/// Measure `comp`'s whole mix in `doc`. `on_progress` is handed how far
/// through it is, from 0 to 1.
pub(crate) fn measure(
    doc: &Arc<Document>,
    comp: Uuid,
    on_progress: &mut dyn FnMut(f64),
) -> Result<Measured, Refused> {
    alone(|| {
        let plan = plan_of(doc, comp)?;
        let m = lumit_render::mix::render::measure(&plan, RATE, &STOP, on_progress)
            .ok_or(Refused::Stopped)?;
        heard(m)
    })
}

/// Measure `comp`'s whole mix in `doc` and find what the master must move
/// by, in dB, for it to measure `lufs`: lifted, measured again, and lifted
/// once more where it is over half a unit off. Answers the move and the mix
/// measured after it.
pub(crate) fn normalise(
    doc: &Arc<Document>,
    comp: Uuid,
    lufs: f64,
    on_progress: &mut dyn FnMut(f64),
) -> Result<(f64, Measured), Refused> {
    alone(|| {
        let mut plan = plan_of(doc, comp)?;
        let (lift, m) =
            land(&mut plan, RATE, lufs, true, &STOP, on_progress).ok_or(Refused::Stopped)?;
        Ok((lift, heard(m)?))
    })
}

/// Run `f` as the one measure running.
fn alone<T>(f: impl FnOnce() -> Result<T, Refused>) -> Result<T, Refused> {
    if MEASURING.swap(true, Ordering::AcqRel) {
        return Err(Refused::Busy);
    }
    STOP.store(false, Ordering::Relaxed);
    let answer = f();
    MEASURING.store(false, Ordering::Release);
    answer
}

/// A measure of a mix with nothing heard in it is no answer.
fn heard(m: Measured) -> Result<Measured, Refused> {
    if m.integrated.is_finite() {
        Ok(m)
    } else {
        Err(Refused::Silent)
    }
}

/// `comp`'s desk built offline as an export builds it.
fn plan_of(doc: &Arc<Document>, comp: Uuid) -> Result<MixPlan, Refused> {
    let c = doc.comp(comp).ok_or(Refused::Silent)?;
    let jobs = lumit_render::headless::AudioJobsBuilder::new().audio_jobs(doc, c);
    if jobs.is_empty() {
        return Err(Refused::Silent);
    }
    let pool = lumit_audio::stream::pool();
    let (plan, _) = build_graph(
        doc,
        c,
        jobs,
        &|job| Some(pool.source(&job.sound, RATE)),
        RATE,
        c.duration.0.to_f64(),
        &mut lumit_render::mix::live::RackBakes::default(),
        None,
        true,
        &STOP,
        None,
    )
    .map_err(|_| Refused::Stopped)?;
    Ok(plan)
}

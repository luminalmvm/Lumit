//! The beat-detection worker.
//!
//! # In plain terms
//!
//! Finding the beat in a composition means mixing down everything audible in
//! it — which decodes every audio file it holds — and then running the onset
//! analysis over the result (docs/impl/beat-detection.md). On a long comp that
//! is seconds of solid work.
//!
//! It used to run wherever the call landed. That is not Dart's own thread —
//! `detect_beats` is an asynchronous call, so it ran on the pool
//! flutter_rust_bridge keeps for them — but that pool is *shared*: it is also
//! how the Project panel gets its thumbnails, how a layer asks whether its
//! source has sound, how the footage panel reads a file's statistics. Two or
//! three detections at once could sit on the whole pool for seconds, and every
//! panel behind them stopped. Asking twice cost twice, and closing the project
//! cancelled nothing, because there was nothing to cancel it *at*.
//!
//! So detection now has a thread of its own, in the shape footage probing
//! uses ([`crate::probe`]):
//!
//! - **One worker, one analysis at a time.** Requests queue in the order they
//!   were made and run on the worker; the caller waits for its own answer, so
//!   `detect_beats` still returns the count it always did, but it waits on a
//!   channel instead of holding a pool thread busy with FFmpeg and FFTs.
//! - **Queued work is cancellable.** Every job carries the generation it was
//!   made in; closing a project bumps it, and the worker drops jobs from a
//!   generation that has ended rather than analysing audio for a document
//!   nobody has open. Their callers are told the project is gone, which by
//!   then it is.
//! - **A fallback that keeps the behaviour.** If the worker thread cannot be
//!   started, or the queue is already deep, the caller does the analysis
//!   itself, exactly as it did before. The worker decides *where* the work
//!   happens and never what the answer is.
//!
//! Determinism is unaffected and stays checkable: the analysis is a pure
//! function of the mixed samples and the sensitivity, so the same comp at the
//! same setting gives the same onset times and confidences whichever thread
//! runs it (docs/impl/beat-detection.md §5.4). The marker *ids* are minted by
//! the caller afterwards, as they always were — this module answers with times
//! and confidences, which is the part that has to be identical.

use std::sync::{
    atomic::{AtomicU64, Ordering},
    mpsc::{channel, Receiver, Sender},
    Arc, Mutex, OnceLock,
};

use uuid::Uuid;

use crate::api::BridgeError;

/// One detected beat: when it lands, and how sure the analysis is.
///
/// Deliberately not a `Marker`: a marker carries an id, ids are minted fresh,
/// and "the same audio gives the same answer" is a claim about times and
/// confidences. Keeping the id out of here keeps that claim testable.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Beat {
    pub time_seconds: f64,
    pub confidence: f32,
}

/// The rate everything is mixed down and analysed at (docs/impl/beat-detection.md
/// §1: mono/stereo f32 at 48 kHz).
const RATE: u32 = 48_000;

/// How many analyses may be waiting for the worker. Past this the caller does
/// its own rather than queueing without bound (docs/14 §5) — the work still
/// happens, it simply happens where it was asked for.
const MAX_QUEUED: usize = 8;

/// What the worker is asked to do, and where to send the answer.
struct Job {
    document: Arc<lumit_core::Document>,
    comp: Uuid,
    duration_seconds: f64,
    sensitivity_percent: u32,
    generation: u64,
    reply: Sender<Result<Vec<Beat>, BridgeError>>,
}

/// The generation queued work belongs to. [`clear`] bumps it and the worker
/// drops anything older — the cancellation rule (docs/14 §6).
fn generation() -> &'static AtomicU64 {
    static GENERATION: AtomicU64 = AtomicU64::new(0);
    &GENERATION
}

/// How many jobs are queued or running, so the queue can be bounded without
/// asking the channel (which cannot be asked).
fn depth() -> &'static Mutex<usize> {
    static DEPTH: OnceLock<Mutex<usize>> = OnceLock::new();
    DEPTH.get_or_init(|| Mutex::new(0))
}

/// The worker's job channel, and with it the thread. Built on the first
/// detection, so a session that never asks for beats never starts it.
fn jobs() -> Option<&'static Sender<Job>> {
    static JOBS: OnceLock<Option<Sender<Job>>> = OnceLock::new();
    JOBS.get_or_init(|| {
        let (tx, rx) = channel::<Job>();
        std::thread::Builder::new()
            .name("lumit-beats".into())
            .spawn(move || run(&rx))
            .ok()
            .map(|_| tx)
    })
    .as_ref()
}

/// The worker loop. Ends when the sender is dropped, which is process exit.
fn run(rx: &Receiver<Job>) {
    while let Ok(job) = rx.recv() {
        let answer = if job.generation == generation().load(Ordering::Relaxed) {
            analyse(
                &job.document,
                job.comp,
                job.duration_seconds,
                job.sensitivity_percent,
            )
        } else {
            // The project this was asked for has closed. Nothing is analysed,
            // and the caller — if one is still waiting — is told why.
            Err(BridgeError::InvalidProject)
        };
        // Off the queue *before* the answer goes out, so a caller that has its
        // answer is a caller whose job is finished by every measure — the
        // other order let a test (and a `MAX_QUEUED` check) see a job that had
        // already replied still counted as waiting.
        if let Ok(mut held) = depth().lock() {
            *held = held.saturating_sub(1);
        }
        // A caller that has gone away drops the receiver; that is not a
        // failure, it is the answer being no longer wanted.
        let _ = job.reply.send(answer);
    }
}

/// Mix the composition's audible sources down and run the onset analysis over
/// them. The whole cost of a detection, and a pure function of its inputs.
///
/// Built through the same headless input path the exporter uses, so what is
/// analysed is what will be exported.
fn analyse(
    document: &lumit_core::Document,
    comp: Uuid,
    duration_seconds: f64,
    sensitivity_percent: u32,
) -> Result<Vec<Beat>, BridgeError> {
    let inputs =
        crate::render::with_export_inputs(document, comp).ok_or(BridgeError::NoAudioPipeline)?;
    if inputs.audio.is_empty() {
        return Err(BridgeError::NoAudio);
    }

    let samples = lumit_render::export::mixdown(&inputs.audio, RATE, duration_seconds);
    let delta = lumit_audio::beat::delta_from_sensitivity(sensitivity_percent.clamp(0, 100) as u8);
    let analysis = lumit_audio::beat::analyse_stereo(&samples, RATE, delta);

    // Snapping pulls onsets that are nearly on the tempo grid onto it, so a
    // performance that drifts by a few milliseconds still cuts cleanly. The
    // 45 ms window is the egui frontend's, kept identical on purpose.
    let times: Vec<f64> = analysis.onsets.iter().map(|o| o.time).collect();
    let snapped = lumit_audio::beat::snap_to_grid(&times, analysis.bpm, 0.045);

    Ok(snapped
        .iter()
        .zip(&analysis.onsets)
        .map(|(time, onset)| Beat {
            time_seconds: *time,
            confidence: onset.confidence,
        })
        .collect())
}

/// Detect `comp`'s beats, on the worker where there is one to use.
///
/// The caller waits for its own answer, so this reads like the synchronous call
/// it replaces; what has changed is which thread spends the seconds. Falls back
/// to analysing here when there is no worker to hand it to or the queue is
/// already deep — the same "never a change of behaviour" fallback footage
/// probing has.
pub(crate) fn detect(
    document: Arc<lumit_core::Document>,
    comp: Uuid,
    duration_seconds: f64,
    sensitivity_percent: u32,
) -> Result<Vec<Beat>, BridgeError> {
    let queued = {
        let Ok(mut held) = depth().lock() else {
            return analyse(&document, comp, duration_seconds, sensitivity_percent);
        };
        if *held >= MAX_QUEUED {
            None
        } else {
            *held += 1;
            Some(())
        }
    };
    if queued.is_none() {
        return analyse(&document, comp, duration_seconds, sensitivity_percent);
    }

    let (reply, answer) = channel();
    let job = Job {
        document: Arc::clone(&document),
        comp,
        duration_seconds,
        sensitivity_percent,
        generation: generation().load(Ordering::Relaxed),
        reply,
    };

    let sent = match jobs() {
        Some(tx) => tx.send(job).is_ok(),
        None => false,
    };
    if !sent {
        if let Ok(mut held) = depth().lock() {
            *held = held.saturating_sub(1);
        }
        return analyse(&document, comp, duration_seconds, sensitivity_percent);
    }

    // A worker that died mid-job drops the sender; the caller then does the
    // work itself rather than reporting a failure it could still avoid.
    match answer.recv() {
        Ok(result) => result,
        Err(_) => analyse(&document, comp, duration_seconds, sensitivity_percent),
    }
}

/// Cancel whatever is queued. Called when a project closes: a detection of a
/// document nobody has open is CPU spent on nothing.
pub(crate) fn clear() {
    generation().fetch_add(1, Ordering::Relaxed);
}

/// How many jobs are queued or running. Tests only.
#[cfg(test)]
pub(crate) fn queue_depth() -> usize {
    depth().lock().map(|held| *held).unwrap_or(0)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// One at a time. The worker, its queue and the generation are all
    /// process-wide, so two of these overlapping could have one cancel the
    /// other's job or read its depth.
    fn serially() -> std::sync::MutexGuard<'static, ()> {
        static SERIAL: Mutex<()> = Mutex::new(());
        SERIAL.lock().unwrap_or_else(|held| held.into_inner())
    }

    /// A composition with no audible source answers "no audio" rather than
    /// analysing silence — and it answers it through the worker, which is the
    /// path being proved here. The queue is back to empty afterwards, so the
    /// worker really did take the job and really did finish it.
    #[test]
    fn a_silent_comp_is_answered_by_the_worker() {
        let _serial = serially();
        let document = Arc::new(lumit_core::Document::new());
        let answer = detect(document, Uuid::now_v7(), 1.0, 50);
        assert!(
            matches!(
                answer,
                Err(BridgeError::NoAudio) | Err(BridgeError::NoAudioPipeline)
            ),
            "a comp with nothing to hear has no beats to find"
        );
        assert_eq!(queue_depth(), 0, "the job is done and off the queue");
    }

    /// The same comp analysed twice gives the same answer. The claim the
    /// threading must not break (docs/impl/beat-detection.md §5.4): where the
    /// analysis runs cannot change what it finds.
    #[test]
    fn the_same_input_gives_the_same_answer() {
        let _serial = serially();
        let document = Arc::new(lumit_core::Document::new());
        let comp = Uuid::now_v7();
        let first = detect(Arc::clone(&document), comp, 1.0, 50);
        let second = detect(document, comp, 1.0, 50);
        assert_eq!(
            first.is_ok(),
            second.is_ok(),
            "two runs of one input agree about whether there was anything to find"
        );
        if let (Ok(first), Ok(second)) = (first, second) {
            assert_eq!(first, second);
        }
    }

    /// Analysing on the worker and analysing inline are the same analysis —
    /// which is what makes the fallback a fallback rather than a second
    /// implementation.
    #[test]
    fn the_fallback_is_the_same_analysis() {
        let _serial = serially();
        let document = Arc::new(lumit_core::Document::new());
        let comp = Uuid::now_v7();
        let through_worker = detect(Arc::clone(&document), comp, 1.0, 50);
        let inline = analyse(&document, comp, 1.0, 50);
        assert_eq!(through_worker.is_ok(), inline.is_ok());
        assert_eq!(
            format!("{through_worker:?}"),
            format!("{inline:?}"),
            "the worker answers what the caller would have found itself"
        );
    }

    /// Closing a project cancels queued detections: a job stamped with a
    /// generation that has ended is dropped rather than analysed, and its
    /// caller is told the project it named is gone.
    #[test]
    fn a_job_from_a_closed_project_is_dropped() {
        let _serial = serially();
        let (reply, answer) = channel();
        let job = Job {
            document: Arc::new(lumit_core::Document::new()),
            comp: Uuid::now_v7(),
            duration_seconds: 1.0,
            sensitivity_percent: 50,
            // One behind whatever the current generation is: exactly what a
            // job queued before a `clear` looks like to the worker.
            generation: generation().load(Ordering::Relaxed).wrapping_sub(1),
            reply,
        };
        if let Ok(mut held) = depth().lock() {
            *held += 1;
        }
        let Some(tx) = jobs() else {
            return; // no worker thread on this machine; nothing to prove
        };
        assert!(tx.send(job).is_ok());
        assert!(matches!(
            answer.recv(),
            Ok(Err(BridgeError::InvalidProject))
        ));
        assert_eq!(queue_depth(), 0);
    }

    /// `clear` moves the generation on, which is the whole of what cancelling
    /// queued work is.
    #[test]
    fn clearing_moves_the_generation_on() {
        let _serial = serially();
        let before = generation().load(Ordering::Relaxed);
        clear();
        assert_ne!(generation().load(Ordering::Relaxed), before);
    }
}

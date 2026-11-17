//! Lining a multicam comp's files up by their sound (`lumit_audio::sync`).
//!
//! Runs on the job's thread, and decodes a few files at once on threads of
//! its own: decoding is most of the wait.

use std::sync::atomic::AtomicUsize;

use lumit_audio::sync;
use lumit_media::audio::SoundSource;

use super::*;

/// Line `files` up by their sound, filling in each one's start, whether it
/// was placed on weak evidence, and a sound file's speed. A file with no
/// sound, or one that will not decode, stays unplaced. `None` when cancelled.
#[frb(ignore)]
pub(super) fn sync(
    files: &[File],
    found: &mut [Found],
    drift: bool,
    cancel: &AtomicBool,
    tell: &(dyn Fn(f32) + Sync),
) -> Option<()> {
    let cancelled = || cancel.load(Ordering::Relaxed);
    // The file's first sound, which is what a clip of it plays unless told
    // otherwise.
    let sounds: Vec<Option<SoundSource>> = files
        .iter()
        .map(|f| {
            let src = f.src.as_ref()?;
            let info = probed(src)?;
            info.audio.as_ref()?;
            Some(SoundSource::file(src.path.clone()))
        })
        .collect();
    let total = sounds.iter().flatten().count().max(1);
    let lanes = std::thread::available_parallelism()
        .map_or(2, std::num::NonZero::get)
        .clamp(1, 4);
    let (next, done) = (AtomicUsize::new(0), AtomicUsize::new(0));
    let heard: Vec<Mutex<Option<sync::Onsets>>> = sounds.iter().map(|_| Mutex::new(None)).collect();
    std::thread::scope(|scope| {
        for _ in 0..lanes {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(sound) = sounds.get(i) else {
                    break;
                };
                if cancelled() {
                    break;
                }
                let Some(sound) = sound else {
                    continue;
                };
                if let (Ok(onsets), Some(slot)) = (sync::analyse(sound, &cancelled), heard.get(i)) {
                    *slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(onsets);
                }
                let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                tell(0.05 + 0.65 * n as f32 / total as f32);
            });
        }
    });
    if cancelled() {
        return None;
    }
    let heard: Vec<Option<sync::Onsets>> = heard
        .into_iter()
        .map(|slot| slot.into_inner().unwrap_or_else(|p| p.into_inner()))
        .collect();
    // The files that were heard, in their own order, each in its device's
    // group.
    let mut which: Vec<usize> = Vec::new();
    let mut inputs: Vec<sync::Input> = Vec::new();
    let mut windows = sync::FileWindows { sounds: Vec::new() };
    for (i, onsets) in heard.iter().enumerate() {
        let (Some(onsets), Some(Some(sound))) = (onsets, sounds.get(i)) else {
            continue;
        };
        let Some(file) = files.get(i) else {
            continue;
        };
        inputs.push(sync::Input {
            onsets,
            group: file.group,
            order: u32::try_from(i).unwrap_or(u32::MAX),
            // A camera keeps its speed, so its clock is the one to keep.
            steady: found.get(i).is_some_and(|f| f.picture.is_some()),
        });
        windows.sounds.push(sound.clone());
        which.push(i);
    }
    let mut progress = |p: f32| tell(0.7 + 0.3 * p);
    let placed = match sync::solve(
        &inputs,
        &windows,
        sync::Options { drift },
        &mut progress,
        &cancelled,
    ) {
        Ok(placed) => placed,
        Err(sync::SyncError::Cancelled) => return None,
        // Nothing could be compared: every file stays unplaced.
        Err(_) => return Some(()),
    };
    for (k, at) in placed.iter().enumerate() {
        let Some(f) = which.get(k).and_then(|i| found.get_mut(*i)) else {
            continue;
        };
        f.uncertain = at.state == sync::State::Uncertain;
        // A sound file that runs fast or slow plays at the speed that keeps
        // it in step. A picture is never retimed: its own clock may not drift
        // as its sound does, so its start moves to share the error out.
        let start = if drift && f.picture.is_none() && at.speed.is_finite() && at.speed > 0.0 {
            f.speed = at.speed;
            at.start
        } else {
            centred(&placed, &inputs, k)
        };
        f.start = start.and_then(|s| Rational::from_f64_on_grid(s, Rational::FLICK_DEN).ok());
    }
    Some(())
}

/// Where input `k` starts when it plays at speed 1 and not at the speed it
/// was measured at: in step at the middle of the stretch it shares with other
/// devices, and out by half the drift at each end of it. Its own time `t`
/// belongs at `start + t / speed` on the shared clock, so the start that puts
/// the middle `m` right is `m - (m - start) * speed`.
#[frb(ignore)]
fn centred(placed: &[sync::Placed], inputs: &[sync::Input], k: usize) -> Option<f64> {
    let span = |j: usize| {
        let (at, input) = (placed.get(j)?, inputs.get(j)?);
        let start = at.start?;
        Some((start, start + input.onsets.seconds / at.speed))
    };
    let (start, end) = span(k)?;
    let (speed, group) = (placed.get(k)?.speed, inputs.get(k)?.group);
    if !speed.is_finite() || speed <= 0.0 {
        return Some(start);
    }
    // What it shares with the others, else the whole of it.
    let (lo, hi) = (0..placed.len())
        .filter(|&j| inputs.get(j).is_some_and(|i| i.group != group))
        .filter_map(span)
        .map(|(s, e)| (s.max(start), e.min(end)))
        .filter(|(s, e)| s < e)
        .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)))
        .unwrap_or((start, end));
    let middle = (lo + hi) / 2.0;
    Some(middle - (middle - start) * speed)
}

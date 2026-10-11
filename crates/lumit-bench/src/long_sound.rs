//! The long cut's sound (docs/13-PERFORMANCE-RULES.md §2, B25 to B29).
//!
//! # In plain terms
//!
//! [`crate::long`] asks what a long cut costs around the picture. These ask
//! the same of its sound, against the same composition: how long the mix
//! takes to plan after an edit, what a second of it costs to mix, how much
//! decoded sound is in memory once the comp is open and once a minute of it
//! has played, and how long a file nobody has seen takes to show a waveform.
//!
//! No sound card is opened. The scenarios drive the plan the way the engine's
//! two threads do: [`MixPlan::fill_step`] is the thread that fills ahead of
//! the playhead and [`MixPlan::mix_into`] is the callback, with the engine's
//! own figures for how far ahead to fill. The decoded blocks go into a pool
//! of the scenario's own, so what is counted is this comp's sound and nothing
//! another scenario left behind.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use lumit_audio::mix::MixPlan;
use lumit_audio::stream::{self, Pool};
use lumit_core::model::Document;
use lumit_render::export::{live_plan, AudioJob, RackBakes};
use lumit_render::headless::AudioJobsBuilder;
use uuid::Uuid;

use crate::long_comp::{self, FPS, FRAMES, MONTAGE_START};
use crate::scenarios::{elapsed_ms, p95, scatter, Measurement};

/// The rate the sound is mixed at: the fixture's own, so nothing is resampled
/// that a sound card at the usual rate would not resample either.
const RATE: u32 = 48_000;
/// Plans built for B25.
const PLAN_SAMPLES: usize = 20;
/// Places a second is mixed at for B26, spread over the two hours.
const MIX_SAMPLES: usize = 20;
/// Frames to a buffer, as a sound card at [`RATE`] asks for ten milliseconds.
const BUFFER_FRAMES: usize = 480;
/// How long B28 plays for, in seconds.
const PLAY_SECONDS: usize = 60;
/// Cold peak builds timed for B29, of which the fastest is kept, as B18 keeps
/// the fastest of its opens.
const PEAK_REPEATS: usize = 3;
/// How long a block the scenarios have had decoded stays wanted, in
/// milliseconds on [`stream::now`]'s clock: the filling thread's own figure.
const HOLD_MS: u64 = 2_000;

/// The long-form comp's sound, ready to be driven by any scenario.
pub struct SoundHarness {
    doc: Arc<Document>,
    comp: Uuid,
    /// One file of the comp, for the peaks.
    file: PathBuf,
    scratch: PathBuf,
}

impl SoundHarness {
    /// Generate (or reuse) the media in `media_dir` and build the long-form
    /// comp over it. `Err` when the media cannot be made.
    pub fn new(media_dir: &Path) -> Result<Self, String> {
        let media = crate::media::generate_long(media_dir)?;
        let (doc, comp) = long_comp::build(&media)?;
        let file = media
            .files
            .first()
            .cloned()
            .ok_or("the long comp has no files")?;
        Ok(Self {
            doc: Arc::new(doc),
            comp,
            file,
            scratch: media_dir.join("long_sound_peaks"),
        })
    }

    /// Every scenario in order, each reported to `progress` as it lands.
    pub fn all(&self, progress: &mut dyn FnMut(Measurement)) -> Result<Vec<Measurement>, String> {
        let mut out = Vec::new();
        let mut land = |ms: Vec<Measurement>| {
            for m in ms {
                progress(m);
                out.push(m);
            }
        };
        land(vec![self.b25_plan_build()?]);
        land(vec![self.b26_mix_one_second()?]);
        land(self.b27_b28_resident_sound()?.to_vec());
        land(vec![self.b29_first_peaks()?]);
        Ok(out)
    }

    /// The comp's audio jobs, with every file already asked whether it has
    /// sound: that question is put once per file per run of the application,
    /// and no edit pays it again.
    fn jobs(&self, builder: &mut AudioJobsBuilder) -> Result<Vec<AudioJob>, String> {
        let comp = self
            .doc
            .comp(self.comp)
            .ok_or("the long comp is not in its document")?;
        let jobs = builder.audio_jobs(&self.doc, comp);
        if jobs.is_empty() {
            return Err("the long comp makes no sound".into());
        }
        Ok(jobs)
    }

    /// The live plan for `jobs`, its files read through `pool`.
    fn plan(pool: &Arc<Pool>, jobs: &[AudioJob]) -> Arc<MixPlan> {
        let seconds = FRAMES as f64 / FPS as f64;
        let (plan, _) = live_plan(
            jobs,
            &|job| Some(pool.source(&job.path, RATE)),
            RATE,
            seconds,
            0.0,
            &mut RackBakes::default(),
        );
        plan
    }

    /// **B25**: the mix planned again after an edit.
    ///
    /// What the prepare worker does when the document changes: walk the comp
    /// for the jobs that sound, then place them as a plan. It is what stands
    /// between an edit and hearing it, and between pressing play on a comp
    /// not yet prepared and its first sound. Nothing is decoded: a plan names
    /// its files.
    pub fn b25_plan_build(&self) -> Result<Measurement, String> {
        let mut builder = AudioJobsBuilder::new();
        self.jobs(&mut builder)?;
        let pool = Pool::new(stream::DEFAULT_BUDGET_BYTES);
        let mut ms = Vec::with_capacity(PLAN_SAMPLES);
        for _ in 0..PLAN_SAMPLES {
            let t = Instant::now();
            let jobs = self.jobs(&mut builder)?;
            std::hint::black_box(Self::plan(&pool, &jobs));
            ms.push(elapsed_ms(t));
        }
        Ok(Measurement {
            budget: "B25",
            value_ms: p95(&mut ms),
            frames: PLAN_SAMPLES as u64,
        })
    }

    /// **B26**: one second of the mix, as the callback mixes it.
    ///
    /// A hundred buffers of ten milliseconds from each of a spread of places,
    /// metered as the callback meters, over sound that is already decoded.
    /// The number is milliseconds of work per second of sound, so 10 is one
    /// per cent of a core.
    pub fn b26_mix_one_second(&self) -> Result<Measurement, String> {
        let mut builder = AudioJobsBuilder::new();
        let pool = Pool::new(stream::DEFAULT_BUDGET_BYTES);
        let plan = Self::plan(&pool, &self.jobs(&mut builder)?);
        let second = RATE as usize;
        let last = (plan.total_frames / second).saturating_sub(1);
        let mut out = vec![0.0f32; BUFFER_FRAMES * 2];
        let mut ms = Vec::with_capacity(MIX_SAMPLES);
        for i in 0..MIX_SAMPLES {
            let from = scatter(i, 0, last) as usize * second;
            plan.warm(from, from + second);
            let t = Instant::now();
            let mut missed = 0;
            for buffer in 0..second / BUFFER_FRAMES {
                let mut acc = [lumit_audio::meter::MeterAcc::default(); lumit_audio::meter::SLOTS];
                missed += plan.mix_into(from + buffer * BUFFER_FRAMES, &mut out, Some(&mut acc));
                std::hint::black_box(&out);
            }
            ms.push(elapsed_ms(t));
            if missed > 0 {
                return Err(format!("B26: {missed} frames were not decoded at {from}"));
            }
        }
        Ok(Measurement {
            budget: "B26",
            value_ms: p95(&mut ms),
            frames: MIX_SAMPLES as u64,
        })
    }

    /// **B27** and **B28**: decoded sound in memory, in megabytes, once the
    /// comp is open and once a minute of it has played.
    ///
    /// B27 is the comp opened and stopped at the head of the montage: the
    /// plan built and the seconds ahead of the playhead filled, as the
    /// engine fills them while stopped so the first press of play starts with
    /// sound. B28 then plays a minute, a second at a time: fill as far ahead
    /// as the engine does while playing, then mix the second. Neither is a
    /// time: see [`Measurement`]. A frame that was due and not decoded fails
    /// the scenario, since memory saved by playing silence is not a saving.
    pub fn b27_b28_resident_sound(&self) -> Result<[Measurement; 2], String> {
        let megabytes = |pool: &Pool| pool.resident_bytes() as f64 / (1024.0 * 1024.0);
        let hold = || stream::now().saturating_add(HOLD_MS);
        let mut builder = AudioJobsBuilder::new();
        let pool = Pool::new(stream::DEFAULT_BUDGET_BYTES);
        let plan = Self::plan(&pool, &self.jobs(&mut builder)?);
        let second = RATE as usize;
        let start = (MONTAGE_START / FPS) as usize;

        let mut blocks = 0u64;
        while plan.fill_step(
            start * second,
            (start + lumit_audio::STOPPED_AHEAD_SECONDS) * second,
            hold(),
        ) {
            blocks += 1;
        }
        let b27 = Measurement {
            budget: "B27",
            value_ms: megabytes(&pool),
            frames: blocks,
        };

        let mut out = vec![0.0f32; second * 2];
        let mut missed = 0;
        for s in start..start + PLAY_SECONDS {
            while plan.fill_step(
                s * second,
                (s + lumit_audio::PLAY_AHEAD_SECONDS) * second,
                hold(),
            ) {}
            missed += plan.mix_into(s * second, &mut out, None);
        }
        if missed > 0 {
            return Err(format!("B28: {missed} frames were due and not decoded"));
        }
        let b28 = Measurement {
            budget: "B28",
            value_ms: megabytes(&pool),
            frames: PLAY_SECONDS as u64,
        };
        Ok([b27, b28])
    }

    /// **B29**: the first peaks of a file nobody has summarised.
    ///
    /// What a waveform lane waits for the first time it shows a source: the
    /// file decoded through once and summarised as it goes, with the peak
    /// file written at the end. Into an empty cache each time, so nothing is
    /// read back. Every later ask, in this run or the next, is a read of that
    /// small file.
    pub fn b29_first_peaks(&self) -> Result<Measurement, String> {
        let mut best = f64::INFINITY;
        for i in 0..PEAK_REPEATS {
            let cache = self.scratch.join(format!("cold_{i}"));
            // Left over from a run that was stopped: it has to be cold.
            let _ = std::fs::remove_dir_all(&cache);
            let t = Instant::now();
            let pyramid = lumit_audio::peaks::load_or_build(&self.file, RATE, Some(&cache));
            best = best.min(elapsed_ms(t));
            let _ = std::fs::remove_dir_all(&cache);
            if pyramid.is_none() {
                return Err(format!("B29: {} would not summarise", self.file.display()));
            }
        }
        let _ = std::fs::remove_dir_all(&self.scratch);
        Ok(Measurement {
            budget: "B29",
            value_ms: best,
            frames: PEAK_REPEATS as u64,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "a timing run: cargo test --release -p lumit-bench -- --ignored long_sound"]
    fn long_sound_budgets() {
        SoundHarness::new(&std::env::temp_dir().join("lumit-bench-media"))
            .unwrap()
            .all(&mut |m| eprintln!("{m:?}"))
            .unwrap();
    }
}

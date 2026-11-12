//! A plan played from frame 0 in order, its racks run offline: what an
//! export writes and every mixdown reads, so a file is the preview's own
//! arithmetic (docs/09-AUDIO.md).
//!
//! Runs on whichever thread asks (an export's, a measure's), never the
//! callback. Waits for decodes and plugins as long as they take.

use lumit_audio::block::{MixWork, SCRATCH_FRAMES};
use lumit_audio::plan::{Fader, MixPlan};
use lumit_audio::rack::RackPuller;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// How many frames are mixed at a time between progress reports.
const STEP_FRAMES: usize = 64 * SCRATCH_FRAMES;

/// One run through a plan from frame 0.
pub struct Render {
    plan: Arc<MixPlan>,
    work: MixWork,
    racks: RackPuller,
    rate: u32,
    /// The next frame to mix.
    at: usize,
    /// Frames the limiter still owes: the master comes out this late, so the
    /// first ones mixed are not the mix.
    owed: usize,
}

impl Render {
    /// A run through `plan` at `rate`, the master through the limiter when
    /// `limit`, else through a plain clamp at the plan's ceiling.
    #[must_use]
    pub fn new(plan: Arc<MixPlan>, rate: u32, limit: bool) -> Self {
        let mut work = MixWork::new(rate);
        work.set_limiter(limit);
        let owed = work.latency() as usize;
        Self {
            plan,
            work,
            racks: RackPuller::new(rate, true),
            rate,
            at: 0,
            owed,
        }
    }

    /// The next frames of the mix into `out` (interleaved stereo,
    /// overwritten).
    pub fn fill(&mut self, out: &mut [f32]) {
        let mut spare = [0.0f32; SCRATCH_FRAMES * 2];
        while self.owed > 0 {
            let n = self.owed.min(SCRATCH_FRAMES);
            self.mix(&mut spare[..n * 2]);
            self.owed -= n;
        }
        self.mix(out);
    }

    fn mix(&mut self, out: &mut [f32]) {
        let frames = out.len() / 2;
        // Racks read ahead of the mix by their delay; a second covers it.
        exact(&self.plan, self.at, self.at + frames + self.rate as usize);
        self.plan
            .mix_block(self.at, out, &mut self.work, &mut self.racks, None, None);
        self.at += frames;
    }
}

/// Have every file sound in `[from, to)` in memory as a decode of the whole
/// file holds it, so an export's samples never depend on how the playback
/// that came before happened to decode them.
fn exact(plan: &MixPlan, from: usize, to: usize) {
    for clip in plan
        .clips
        .iter()
        .filter(|c| !c.silent && c.source.is_file())
    {
        let a = (from as i64).max(clip.start_frame);
        let b = (to as i64).min(clip.start_frame.saturating_add(clip.len as i64));
        if b > a {
            let first = clip.src_start + (a - clip.start_frame) as usize;
            let _ = clip.source.read(first, (b - a) as usize, true);
        }
    }
}

/// The whole of `plan`, `total_frames` long, as [`Render`] plays it.
/// `on_progress` is handed how far through it is, from 0 to 1.
#[must_use]
pub fn render(
    plan: Arc<MixPlan>,
    rate: u32,
    limit: bool,
    on_progress: &mut dyn FnMut(f64),
) -> Vec<f32> {
    let total = plan.total_frames;
    let mut out = vec![0.0f32; total * 2];
    let mut run = Render::new(plan, rate, limit);
    let mut done = 0usize;
    for chunk in out.chunks_mut(STEP_FRAMES * 2) {
        run.fill(chunk);
        done += chunk.len() / 2;
        on_progress(done as f64 / total.max(1) as f64);
    }
    out
}

/// A mix measured as it is heard.
#[derive(Debug, Clone, Copy)]
pub struct Measured {
    /// Integrated loudness, in LUFS. Minus infinity for a silent mix.
    pub integrated: f64,
    /// The loudest true peak, in dBTP.
    pub true_peak_db: f64,
    /// Loudness range, in LU.
    pub range: f64,
    /// The loudest short-term reading, in LUFS.
    pub short_term_max: f64,
}

/// `plan`'s master measured as playback and an export hear it: through the
/// limiter at the plan's ceiling. None once `stop` is raised. `on_progress`
/// as [`render`]'s.
#[must_use]
pub fn measure(
    plan: &MixPlan,
    rate: u32,
    stop: &AtomicBool,
    on_progress: &mut dyn FnMut(f64),
) -> Option<Measured> {
    // A tenth of a second at a time, the step the short-term reading moves by.
    let step = (rate as usize / 10).max(1);
    let total = plan.total_frames;
    let mut run = Render::new(Arc::new(plan.clone()), rate, true);
    let mut meter = lumit_audio::loudness::Loudness::new(rate);
    let mut chunk = vec![0.0f32; step * 2];
    let mut short_term_max = f64::NEG_INFINITY;
    let mut done = 0usize;
    let mut told = 0usize;
    while done < total {
        if stop.load(Ordering::Relaxed) {
            return None;
        }
        let n = (total - done).min(step);
        run.fill(&mut chunk[..n * 2]);
        meter.add(&chunk[..n * 2]);
        short_term_max = short_term_max.max(f64::from(meter.reading().short_term));
        done += n;
        // Told each hundredth, not each step.
        let now = done * 100 / total;
        if now != told {
            told = now;
            on_progress(done as f64 / total as f64);
        }
    }
    let reading = meter.reading();
    Some(Measured {
        integrated: f64::from(reading.integrated),
        true_peak_db: 20.0 * f64::from(reading.true_peak.max(1e-10)).log10(),
        range: f64::from(reading.range),
        short_term_max,
    })
}

/// How far from a loudness a mix may land, in LU.
const LAND_WITHIN: f64 = 0.5;

/// Move `plan`'s master so the mix, as it is heard, measures `lufs`: lift
/// it by the difference, measure again, and where the limiter took more
/// than half a unit back, lift once more by what the two measures say a
/// decibel buys. With `verify` the mix is measured after that too. Answers
/// the whole lift in dB and the last measure (the one after the lift when
/// verified or not needed, else the one before the correction), or None
/// when stopped. A silent mix keeps its master. `on_progress` as
/// [`render`]'s.
pub fn land(
    plan: &mut MixPlan,
    rate: u32,
    lufs: f64,
    verify: bool,
    stop: &AtomicBool,
    on_progress: &mut dyn FnMut(f64),
) -> Option<(f64, Measured)> {
    let mut pass = 0.0;
    let mut measured = |plan: &MixPlan, pass: &mut f64| {
        let at = *pass;
        *pass += 1.0;
        measure(plan, rate, stop, &mut |f| on_progress((at + f) / 3.0))
    };
    let before = measured(plan, &mut pass)?;
    if !before.integrated.is_finite() {
        on_progress(1.0);
        return Some((0.0, before));
    }
    let first = lufs - before.integrated;
    lift(plan, first);
    let after = measured(plan, &mut pass)?;
    let short = lufs - after.integrated;
    if !after.integrated.is_finite() || short.abs() <= LAND_WITHIN {
        on_progress(1.0);
        return Some((first, after));
    }
    // The limiter gives back less than a decibel for each one put in.
    let per_db = if first.abs() > 1e-6 {
        ((after.integrated - before.integrated) / first).clamp(0.2, 1.0)
    } else {
        1.0
    };
    let more = short / per_db;
    lift(plan, more);
    let last = if verify {
        measured(plan, &mut pass)?
    } else {
        after
    };
    on_progress(1.0);
    Some((first + more, last))
}

/// Set `plan` for a loudness target: the limiter's ceiling at
/// `true_peak_db` and the master moved until the mix lands on `lufs`
/// ([`land`]). None when stopped.
pub fn aim(
    plan: &mut MixPlan,
    rate: u32,
    lufs: f64,
    true_peak_db: f64,
    stop: &AtomicBool,
    on_progress: &mut dyn FnMut(f64),
) -> Option<()> {
    plan.ceiling = lumit_audio::mix::db_to_gain(true_peak_db);
    land(plan, rate, lufs, false, stop, on_progress).map(|_| ())
}

/// `plan`'s master, keys and all, moved by `db`.
pub fn lift(plan: &mut MixPlan, db: f64) {
    scale(&mut plan.master, 10f64.powf(db / 20.0) as f32);
}

/// `fader` turned up by `gain`, keys and all.
fn scale(fader: &mut Fader, gain: f32) {
    fader.gain = fader.gain.map(|g| g * gain);
    if let Some(envelope) = &mut fader.envelope {
        let mut scaled = (**envelope).clone();
        for point in &mut scaled.points {
            *point = point.map(|g| g * gain);
        }
        *envelope = Arc::new(scaled);
    }
}

/// `plan` as one bus's stem: only node `bus` reaches the master, at its own
/// fader, with the master's fader, limiter and ceiling left out. None when
/// the plan will not take it (it always should).
#[must_use]
pub fn stem(plan: &MixPlan, bus: u16) -> Option<MixPlan> {
    let mut stem = plan.clone();
    for (i, node) in stem.nodes.iter_mut().enumerate() {
        if i == usize::from(bus) {
            node.out = None;
        } else if node.out.is_none() {
            node.out_on = false;
        }
    }
    for clip in &mut stem.clips {
        if clip.node == lumit_audio::plan::NONE {
            clip.silent = true;
        }
    }
    stem.master = Fader::default();
    stem.ceiling = f32::MAX;
    stem.finish().ok()
}

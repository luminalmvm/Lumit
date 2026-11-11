//! The clean-up effects' doorway: Denoise's Learn and its sample square, and
//! what the clean-up effects' displays read.
//!
//! # In plain terms
//!
//! Denoise works from a measurement of the noise, and the measurement is made
//! here: the effect's own input is rendered at the playhead, the noise in it is
//! read ([`lumit_core::fx::denoise::learn`]), and what was found is written onto
//! the effect's rows as one undo step. A press of **Learn** finds the flattest
//! patch itself; a square put down on the Viewer is measured where it lies.
//!
//! **What the measure sees is what the effect is handed.** The layer is drawn
//! alone, at its own size, with no transform, mask or matte, and with Denoise
//! and everything after it switched off, so the noise is read in the effect's
//! own pixels rather than in the comp's.
//!
//! **A press never waits.** Learn runs on a thread of its own and the display
//! reads how it is getting on, as the trackers' cards do. The square's measure
//! is an async call, so its caller waits off the interface thread.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use flutter_rust_bridge::frb;
use lumit_core::fx::denoise::learn::{learn, LearnIssue, Learned};
use lumit_core::model::{Document, EffectInstance, ProjectItem, TransformGroup};
use lumit_core::time::Rational;
use uuid::Uuid;

use crate::api::{layer::LayerReference, BridgeError};

/// Denoise's match name.
pub(crate) const DENOISE: &str = "denoise";

/// The Learn button's id on the effects that have one.
pub(crate) const LEARN: &str = "learn";

// ---------------------------------------------------------------------------
// What a display reads
// ---------------------------------------------------------------------------

/// How a measure of one effect's noise went, for its display.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeNoiseStatus {
    /// A measure is running.
    pub busy: bool,
    /// Why the last measure wrote nothing: 0 none, 1 nothing to measure.
    pub failure: u32,
    /// How far the last measure can be trusted, 0 to 1.
    pub quality: f32,
    /// Why the quality is low: 0 it is not, 1 a small sample, 2 detail in it,
    /// 3 clipped, 4 little noise.
    pub reason: u32,
    /// The patch that was measured, straight RGBA8, `side` by `side`, as the
    /// display draws it. Empty until something has been measured.
    pub picture: Vec<u8>,
    pub side: u32,
    /// Counts the measures that have landed, so a display knows to read the
    /// picture again. Nought before the first.
    pub generation: u32,
    /// The square the measure on file read, as the Sample rows held it:
    /// centre x, centre y and side. Noughts before the first.
    pub sample_x: f32,
    pub sample_y: f32,
    pub sample_size: f32,
}

impl BridgeNoiseStatus {
    fn idle() -> Self {
        Self {
            busy: false,
            failure: 0,
            quality: 0.0,
            reason: 0,
            picture: Vec::new(),
            side: 0,
            generation: 0,
            sample_x: 0.0,
            sample_y: 0.0,
            sample_size: 0.0,
        }
    }
}

/// What a measure is filed under: the project and the effect. A saved copy of
/// a project open beside the original holds the same effect ids, and the two
/// must not share a picture.
type Key = (Uuid, Uuid);

fn key(layer: &LayerReference, effect: Uuid) -> Key {
    (layer.project_id(), effect)
}

/// The last measure of each effect.
///
/// Compacts by count: once it holds [`KEPT`] effects the one measured longest
/// ago goes, so a long day adding and removing effects keeps a handful of
/// small pictures and no more.
static STATUS: LazyLock<Mutex<HashMap<Key, BridgeNoiseStatus>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// What is running for each effect: how many measures, and whether one of
/// them is a Learn. A status is busy while any measure it shows is running,
/// so one landing does not say another has.
static RUNNING: LazyLock<Mutex<HashMap<Key, (u32, bool)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Why a measure is being made.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Asked {
    /// A press of Learn. One at a time: a second press while one runs is
    /// refused rather than started again.
    Learn,
    /// A square put down on the Viewer.
    Square,
    /// The display asking what the square stands on. It writes nothing, so it
    /// neither shows as busy nor runs beside another measure.
    Peek,
}

/// Count a measure in. `false` when it must not start.
fn begin(key: Key, asked: Asked) -> bool {
    let Ok(mut all) = RUNNING.lock() else {
        return false;
    };
    let entry = all.entry(key).or_insert((0, false));
    let refused = match asked {
        Asked::Learn => entry.1,
        Asked::Square => false,
        Asked::Peek => entry.0 > 0,
    };
    if refused {
        return false;
    }
    entry.0 += 1;
    entry.1 |= asked == Asked::Learn;
    drop(all);
    if asked != Asked::Peek {
        set_status(key, |s| s.busy = true);
    }
    true
}

/// Count a measure out, and say whether one the status shows is still
/// running.
fn end(key: Key, asked: Asked) -> bool {
    let Ok(mut all) = RUNNING.lock() else {
        return false;
    };
    let Some(entry) = all.get_mut(&key) else {
        return false;
    };
    entry.0 = entry.0.saturating_sub(1);
    if asked == Asked::Learn {
        entry.1 = false;
    }
    let busy = entry.0 > 0;
    if !busy {
        all.remove(&key);
    }
    busy
}

/// How many effects' measures are kept.
const KEPT: usize = 64;

/// The largest side the measured patch is handed to the display at.
const PATCH_SIDE: u32 = 96;

fn set_status(key: Key, edit: impl FnOnce(&mut BridgeNoiseStatus)) {
    let Ok(mut all) = STATUS.lock() else {
        return;
    };
    if !all.contains_key(&key) && all.len() >= KEPT {
        if let Some(oldest) = all
            .iter()
            .filter(|(_, s)| !s.busy)
            .min_by_key(|(_, s)| s.generation)
            .map(|(id, _)| *id)
        {
            all.remove(&oldest);
        }
    }
    edit(all.entry(key).or_insert_with(BridgeNoiseStatus::idle));
}

/// How the last measure of `effect` went. Cheap: a lookup behind a mutex, so
/// a display may ask twice a second while one is running.
#[frb(sync)]
pub fn noise_status(layer: LayerReference, effect: Uuid) -> BridgeNoiseStatus {
    STATUS
        .lock()
        .ok()
        .and_then(|all| all.get(&key(&layer, effect)).cloned())
        .unwrap_or_else(BridgeNoiseStatus::idle)
}

// ---------------------------------------------------------------------------
// The sound clean-up effects' displays
// ---------------------------------------------------------------------------

/// What a sound clean-up effect's display draws at the playhead.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeRepairView {
    /// 0 a spectrum, 1 the hum, 2 the clicks, 3 a spectrum with the room's
    /// reverb time.
    pub kind: u32,
    /// The spectrum's curves, each the engine's points in dB on one log
    /// frequency axis from 20 Hz to 20 kHz, or empty where the effect has no
    /// use for one: what arrives, what the effect works from, where it starts
    /// to act, and what comes out.
    pub input: Vec<f64>,
    pub floor: Vec<f64>,
    pub threshold: Vec<f64>,
    pub output: Vec<f64>,
    /// The hum's frequency in hertz, or the reverb time in seconds; nought
    /// where none was found.
    pub value: f64,
    /// How many harmonics of the hum the effect takes out, how many clicks
    /// there are, or one when Noise reduction is working from a learned
    /// profile.
    pub count: u32,
    /// Each click's start and end as fractions of the window, in pairs.
    pub marks: Vec<f64>,
    /// The window's sound as a minimum and a maximum per column, in pairs.
    pub wave: Vec<f64>,
}

/// Seconds of sound either side of the playhead a spectrum reads, a hum view
/// and a click view. The hum's is the length its finder works on: a shorter
/// window cannot tell a mains harmonic from a peak in the noise.
const SPECTRUM_HALF: f64 = 0.125;
const HUM_HALF: f64 = 0.7;
const CLICKS_HALF: f64 = 1.0;

/// Seconds either side of the playhead the room's reverb time is looked for
/// in: long enough to hold a word ending and the decay after it.
const DECAY_HALF: f64 = 1.5;

/// Columns in a click view's wave.
const WAVE_COLUMNS: usize = 200;

/// What `effect`'s display draws at `frame`, worked out from its rows there and
/// a window of the sound arriving at it. Off the interface thread: a long
/// file's first read decodes it.
pub fn repair_view(
    layer: LayerReference,
    effect: Uuid,
    frame: u64,
) -> Result<BridgeRepairView, BridgeError> {
    use lumit_core::fx::effects::audio::repair::view;

    let project = layer.project()?;
    let doc = project
        .read()
        .map_err(|_| BridgeError::ReadFailed)?
        .store
        .snapshot();
    let comp = layer.composition()?;
    let item = layer.item()?;
    let fx = find(&item, effect).ok_or(BridgeError::InvalidEffect)?;
    let t = comp
        .frame_rate
        .time_of_frame(i64::try_from(frame).map_err(|_| BridgeError::InvalidTime)?)
        .map_err(|_| BridgeError::InvalidTime)?
        .0
        .to_f64();
    let lt = t - item.start_offset.0.to_f64();
    let values = values_at(&fx, lt);
    let half = match fx.effect.match_name.as_str() {
        "audio_declick" => CLICKS_HALF,
        "audio_dehum" => HUM_HALF,
        _ => SPECTRUM_HALF,
    };
    let (window, rate) = crate::sound_span::sound_window(&doc, &comp, &item, t, lt, half);
    let mut out = BridgeRepairView {
        kind: 0,
        input: Vec::new(),
        floor: Vec::new(),
        threshold: Vec::new(),
        output: Vec::new(),
        value: 0.0,
        count: 0,
        marks: Vec::new(),
        wave: Vec::new(),
    };
    if window.is_empty() {
        return Ok(out);
    }
    let spectrum = |out: &mut BridgeRepairView, s: view::SpectrumView| {
        let wide = |v: Vec<f32>| v.into_iter().map(f64::from).collect::<Vec<_>>();
        out.input = wide(s.input);
        out.floor = s.floor.map(wide).unwrap_or_default();
        out.threshold = s.threshold.map(wide).unwrap_or_default();
        out.output = wide(s.output);
    };
    match fx.effect.match_name.as_str() {
        "audio_denoise" => {
            let state = fx.plugin_state_bytes();
            // One when the floor drawn is the learned one.
            let adaptive = row(&values, "adaptive").is_none_or(|on| on > 0.5);
            out.count = u32::from(state.is_some() && !adaptive);
            spectrum(
                &mut out,
                view::noise_reduction(&values, state.as_deref(), &window, rate),
            );
        }
        "audio_dehum" => {
            out.kind = 1;
            // The frequency the effect acts on: the one found in the sound,
            // or the row's own with Find frequency off. The marks are the
            // harmonics it will take out.
            let frequency = if row(&values, "auto").is_none_or(|on| on > 0.5) {
                view::hum_removal(&values, &window, rate)
                    .frequency
                    .map(f64::from)
            } else {
                row(&values, "frequency")
            };
            if let Some(hz) = frequency {
                out.value = hz;
                out.count = row(&values, "harmonics").map_or(0, |h| h.round().max(0.0) as u32);
            }
            // The ground under the marks is the sound coming in.
            spectrum(&mut out, view::de_esser(&values, &window, rate));
            out.threshold.clear();
            out.output.clear();
        }
        "audio_declick" => {
            out.kind = 2;
            let clicks = view::click_removal(&values, &window, rate);
            let n = window.len().max(1) as f64;
            out.count = u32::try_from(clicks.len()).unwrap_or(u32::MAX);
            for (start, end) in clicks {
                out.marks.push(f64::from(start) / n);
                out.marks.push(f64::from(end) / n);
            }
            out.wave = crate::sound_span::columns(&window, WAVE_COLUMNS);
        }
        "audio_deess" => spectrum(&mut out, view::de_esser(&values, &window, rate)),
        "audio_dereverb" => {
            out.kind = 3;
            spectrum(&mut out, view::reverb_reduction(&values, &window, rate));
            let (long, long_rate) =
                crate::sound_span::sound_window(&doc, &comp, &item, t, lt, DECAY_HALF);
            if !long.is_empty() {
                out.value = view::reverb_time(&long, long_rate).map_or(0.0, f64::from);
            }
        }
        _ => return Err(BridgeError::InvalidEffect),
    }
    Ok(out)
}

/// What each of the instance's number, switch and list rows holds at layer
/// time `lt`, as the bake hands them to the effect: a switch as nought or one,
/// a list as the place of its choice.
fn values_at(fx: &EffectInstance, lt: f64) -> Vec<(lumit_core::fx::params::ParamId, f64)> {
    use lumit_core::model::EffectValue;
    let Some(def) = lumit_core::fx::catalogue::BUILTIN_DEFS.get(&fx.effect.match_name) else {
        return Vec::new();
    };
    def.schema()
        .params
        .iter()
        .filter_map(|row| {
            let held = fx.params.iter().find(|p| p.id == row.id)?;
            let value = match &held.value {
                EffectValue::Float(_) => fx.float_at(row.id, lt)?,
                EffectValue::Bool(on) => f64::from(u8::from(*on)),
                EffectValue::Choice(index) => f64::from(*index),
                _ => return None,
            };
            Some((lumit_core::fx::params::ParamId::new(row.id), value))
        })
        .collect()
}

/// What row `id` holds among `values`, or `None` when the effect has no such
/// row.
fn row(values: &[(lumit_core::fx::params::ParamId, f64)], id: &'static str) -> Option<f64> {
    let id = lumit_core::fx::params::ParamId::new(id);
    values.iter().find(|(held, _)| *held == id).map(|(_, v)| *v)
}

// ---------------------------------------------------------------------------
// Learn and the square
// ---------------------------------------------------------------------------

/// A press of one of a clean-up effect's buttons, from `fire_effect_action`:
/// `None` when `effect` is not a clean-up effect, on the layer's own stack or
/// one of its clips', and the press is somebody else's.
pub(crate) fn try_press(
    layer: &LayerReference,
    effect: Uuid,
    param: &str,
    frame: Option<u64>,
) -> Option<Result<(), BridgeError>> {
    let item = layer.item().ok()?;
    let fx = find(&item, effect)?;
    match fx.effect.match_name.as_str() {
        DENOISE => Some(press(layer, &fx, param, frame)),
        NOISE_REDUCTION => Some(press_sound(layer, &fx, param, frame)),
        _ => None,
    }
}

/// Noise reduction's match name.
pub(crate) const NOISE_REDUCTION: &str = "audio_denoise";

/// The longest stretch Learn reads, in seconds, and the stretch it keeps when
/// it has to find the quiet part itself.
const LEARN_MOST: f64 = 30.0;
const LEARN_QUIET: f64 = 1.5;

fn press_sound(
    layer: &LayerReference,
    fx: &EffectInstance,
    param: &str,
    frame: Option<u64>,
) -> Result<(), BridgeError> {
    if param != LEARN {
        return Err(BridgeError::InvalidParam);
    }
    let (layer, effect, frame) = (*layer, fx.id, frame.unwrap_or(0));
    let key = key(&layer, effect);
    if !begin(key, Asked::Learn) {
        return Err(BridgeError::AnalysisBusy);
    }
    std::thread::Builder::new()
        .name("noise learn".to_owned())
        .spawn(move || {
            let learned = learn_sound(&layer, effect, frame);
            let busy = end(key, Asked::Learn);
            set_status(key, |s| {
                s.busy = busy;
                s.failure = u32::from(learned.is_err());
                if learned.is_ok() {
                    s.generation = next_generation();
                }
            });
        })
        .map_err(|_| {
            let busy = end(key, Asked::Learn);
            set_status(key, |s| s.busy = busy);
            BridgeError::AnalysisBusy
        })?;
    Ok(())
}

/// Learn Noise reduction's profile from the sound arriving at it: over the
/// comp's work area where that narrows down the layer, or else from the
/// quietest stretch of the layer around the playhead. Writes the profile and
/// turns Adaptive off, as one undo step.
fn learn_sound(layer: &LayerReference, effect: Uuid, frame: u64) -> Result<(), BridgeError> {
    use lumit_core::fx::effects::audio::repair::denoise::learn_profile;

    let project = layer.project()?;
    let doc = project
        .read()
        .map_err(|_| BridgeError::ReadFailed)?
        .store
        .snapshot();
    let comp = layer.composition()?;
    let item = layer.item()?;
    let fx = find(&item, effect).ok_or(BridgeError::InvalidEffect)?;
    let playhead = comp
        .frame_rate
        .time_of_frame(i64::try_from(frame).map_err(|_| BridgeError::InvalidTime)?)
        .map_err(|_| BridgeError::InvalidTime)?
        .0
        .to_f64();
    // The layer's sounding span, in comp seconds.
    let (from, to) = (item.in_point.0.to_f64(), item.out_point.0.to_f64());
    let narrowed = comp.work_area.and_then(|(a, b)| {
        let (a, b) = (a.0.to_f64().max(from), b.0.to_f64().min(to));
        (b - a > 0.05 && (b - a) < (to - from) - 1e-6).then_some((a, b.min(a + LEARN_MOST)))
    });
    let (a, b) = narrowed.unwrap_or_else(|| {
        let a = (playhead - LEARN_MOST / 2.0).max(from);
        (a, (a + LEARN_MOST).min(to))
    });
    let (mono, rate) = crate::sound_span::read_span(&doc, &comp, &item, a, b);
    if mono.is_empty() || rate == 0 {
        return Err(BridgeError::ReadFailed);
    }
    let mono = if narrowed.is_some() {
        mono
    } else {
        crate::sound_span::quietest(&mono, rate, LEARN_QUIET).ok_or(BridgeError::ReadFailed)?
    };
    let stereo: Vec<f32> = mono.iter().flat_map(|s| [*s, *s]).collect();
    let stereo = crate::sound_span::ahead_of(&doc, &comp, &item, &fx, &stereo, a, rate);
    let blob = learn_profile(&stereo, rate);
    let home = layer.instance_home(&fx)?;
    layer.with_instances(home, |list| {
        let fx = list
            .iter_mut()
            .find(|e| e.id == effect)
            .ok_or(BridgeError::InvalidEffect)?;
        fx.set_plugin_state(&blob);
        if let Some(adaptive) = fx.params.iter_mut().find(|p| p.id == "adaptive") {
            adaptive.value = lumit_core::model::EffectValue::Bool(false);
        }
        Ok(())
    })
}

fn press(
    layer: &LayerReference,
    fx: &EffectInstance,
    param: &str,
    frame: Option<u64>,
) -> Result<(), BridgeError> {
    if param != LEARN {
        return Err(BridgeError::InvalidParam);
    }
    let frame = frame.unwrap_or(0);
    let (layer, effect) = (*layer, fx.id);
    let key = key(&layer, effect);
    if !begin(key, Asked::Learn) {
        return Err(BridgeError::AnalysisBusy);
    }
    std::thread::Builder::new()
        .name("noise learn".to_owned())
        .spawn(move || {
            let _ = measure(&layer, effect, frame, None);
        })
        .map_err(|_| {
            let busy = end(key, Asked::Learn);
            set_status(key, |s| s.busy = busy);
            BridgeError::AnalysisBusy
        })?;
    Ok(())
}

/// Measure the noise in a square put down on the Viewer, and write the square
/// and what was found there as one undo step. `centre_x`, `centre_y` and
/// `side` are in the effect's own pixels, as its Sample rows are.
///
/// Writes the square even when there is nothing to measure there, so a move
/// is never lost; the noise rows then stay as they were and the display says
/// why.
pub fn denoise_measure_at(
    layer: LayerReference,
    effect: Uuid,
    frame: u64,
    centre_x: f64,
    centre_y: f64,
    side: f64,
) -> Result<(), BridgeError> {
    begin(key(&layer, effect), Asked::Square);
    let square = [centre_x as f32, centre_y as f32, side as f32];
    measure(&layer, effect, frame, Some(square))
}

/// The measure itself, on whatever thread called it: render the effect's
/// input, read its noise, write the rows, file the result.
fn measure(
    layer: &LayerReference,
    effect: Uuid,
    frame: u64,
    square: Option<[f32; 3]>,
) -> Result<(), BridgeError> {
    let found = effect_input(layer, effect, frame).map(|(rgba, w, h)| {
        let learned = learn(&rgba, w, h, square);
        // The picture is of where the noise was read, which a press of
        // Learn chooses for itself.
        let picture = patch(&rgba, w, h, learned.sample);
        (learned, picture)
    });
    // A measure that found nothing to read (a black frame, a patch with no
    // noise in it) carries no profile, and must not put one down.
    let usable = found
        .as_ref()
        .is_some_and(|(learned, _)| learned.quality > 0.0);
    let written = match (&found, square) {
        (Some((learned, _)), _) if usable => write(layer, effect, frame, &learned.rows()),
        // Nothing to measure: the square still goes where it was put, and
        // the noise rows stay as they were.
        (_, Some([x, y, side])) => write(
            layer,
            effect,
            frame,
            &[("sample_x", x), ("sample_y", y), ("sample_size", side)],
        ),
        (_, None) => Ok(()),
    };
    let asked = if square.is_some() {
        Asked::Square
    } else {
        Asked::Learn
    };
    let key = key(layer, effect);
    let busy = end(key, asked);
    set_status(key, |s| {
        s.busy = busy;
        match (&found, square) {
            (Some((learned, picture)), _) if usable => file(s, learned, picture, learned.sample),
            // The patch and why it would not do, where a square says which
            // patch; a Learn that found none has only that to say.
            (Some((learned, picture)), Some(square)) => file(s, learned, picture, square),
            _ => s.failure = 1,
        }
    });
    written
}

/// File a landed measure for the display: what was found, the picture of
/// where, and the square the Sample rows now hold.
fn file(s: &mut BridgeNoiseStatus, learned: &Learned, picture: &(Vec<u8>, u32), square: [f32; 3]) {
    s.failure = 0;
    s.quality = learned.quality;
    s.reason = reason_code(learned);
    s.picture.clone_from(&picture.0);
    s.side = picture.1;
    s.generation = next_generation();
    [s.sample_x, s.sample_y, s.sample_size] = square;
}

/// Measure the square as it stands and file the result for the display,
/// writing nothing onto the effect.
///
/// A measure lives only as long as the program does, so an effect in a
/// project just opened has rows and no picture of where they came from; and
/// an undo or a typed row moves the square without measuring it. Either way
/// the display asks here. Nothing happens when the measure on file already
/// reads this square or another measure is running, the status never shows
/// as busy for it, and nothing is said when there is no picture to read: the
/// display stays as it was.
pub fn denoise_peek(layer: LayerReference, effect: Uuid, frame: u64) -> Result<(), BridgeError> {
    let comp = layer.composition()?;
    let item = layer.item()?;
    let fx = find(&item, effect).ok_or(BridgeError::InvalidEffect)?;
    if fx.effect.match_name != DENOISE {
        return Err(BridgeError::InvalidEffect);
    }
    let lt = comp
        .frame_rate
        .time_of_frame(i64::try_from(frame).map_err(|_| BridgeError::InvalidTime)?)
        .map_err(|_| BridgeError::InvalidTime)?
        .0
        .to_f64()
        - item.start_offset.0.to_f64();
    let at = |id: &str| fx.float_at(id, lt).map(|v| v as f32);
    let (Some(x), Some(y), Some(side)) = (at("sample_x"), at("sample_y"), at("sample_size")) else {
        return Ok(());
    };
    let square = [x, y, side];
    let key = key(&layer, effect);
    let on_file = STATUS.lock().ok().is_some_and(|all| {
        all.get(&key).is_some_and(|s| {
            s.generation != 0
                && (s.sample_x - x).abs() < 0.5
                && (s.sample_y - y).abs() < 0.5
                && (s.sample_size - side).abs() < 0.5
        })
    });
    if on_file || !begin(key, Asked::Peek) {
        return Ok(());
    }
    let found = effect_input(&layer, effect, frame).map(|(rgba, w, h)| {
        let learned = learn(&rgba, w, h, Some(square));
        let picture = patch(&rgba, w, h, learned.sample);
        (learned, picture)
    });
    let busy = end(key, Asked::Peek);
    set_status(key, |s| {
        s.busy = busy;
        if let Some((learned, picture)) = &found {
            file(s, learned, picture, square);
        }
    });
    Ok(())
}

/// Counts landed measures across every effect, so each display's generation
/// only ever rises.
fn next_generation() -> u32 {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

fn reason_code(learned: &Learned) -> u32 {
    match learned.reason {
        None => 0,
        Some(LearnIssue::Small) => 1,
        Some(LearnIssue::NotFlat) => 2,
        Some(LearnIssue::Clipped) => 3,
        Some(LearnIssue::TooLittleNoise) => 4,
    }
}

/// Write `rows` onto the effect, wherever it lives, as one undo step. A keyed
/// row takes a key at the playhead, a row an expression drives stays the
/// expression's.
fn write(
    layer: &LayerReference,
    effect: Uuid,
    frame: u64,
    rows: &[(&str, f32)],
) -> Result<(), BridgeError> {
    let comp = layer.composition()?;
    let item = layer.item()?;
    let at = comp
        .frame_rate
        .time_of_frame(i64::try_from(frame).map_err(|_| BridgeError::InvalidTime)?)
        .map_err(|_| BridgeError::InvalidTime)?
        .0
        .checked_sub(item.start_offset.0)
        .unwrap_or(Rational::ZERO);
    let instance = find(&item, effect).ok_or(BridgeError::InvalidEffect)?;
    let home = layer.instance_home(&instance)?;
    layer.with_instances(home, |list| {
        let fx = list
            .iter_mut()
            .find(|e| e.id == effect)
            .ok_or(BridgeError::InvalidEffect)?;
        for (id, value) in rows {
            if let Some(param) = fx.params.iter_mut().find(|p| p.id == *id) {
                crate::api::track::write_row(
                    &mut param.value,
                    lumit_core::fx::params::Value::Float(*value),
                    at,
                );
            }
        }
        Ok(())
    })
}

/// The instance `effect` on `layer`: its own stack, or one of its clips'.
fn find(layer: &lumit_core::model::Layer, effect: Uuid) -> Option<EffectInstance> {
    if let Some(fx) = layer.effects.iter().find(|e| e.id == effect) {
        return Some(fx.clone());
    }
    match &layer.kind {
        lumit_core::model::LayerKind::Sequence { clips } => clips
            .iter()
            .flat_map(|c| c.effects.iter())
            .find(|e| e.id == effect)
            .cloned(),
        _ => None,
    }
}

/// The picture `effect` is handed at `frame`, in the layer's own pixels,
/// scene-linear and premultiplied: the layer drawn alone at its own size, with
/// no transform, mask or matte, and with `effect` and everything after it
/// switched off, on the layer's stack or a clip's rack. `None` with no
/// graphics adapter or nothing drawn.
fn effect_input(layer: &LayerReference, effect: Uuid, frame: u64) -> Option<(Vec<f32>, u32, u32)> {
    let project = layer.project().ok()?;
    let doc = project.read().ok()?.store.snapshot();
    let comp = layer.composition().ok()?;
    let item = layer.item().ok()?;
    let (w, h) = raster_size(&doc, &comp, &item);
    let mut solo = item.clone();
    solo.transform = TransformGroup::default();
    solo.parent = None;
    solo.matte = None;
    solo.masks.clear();
    solo.switches.visible = true;
    solo.switches.solo = false;
    solo.switches.three_d = false;
    solo.switches.motion_blur = false;
    // Off from the effect on, on whichever stack holds it: the layer's own,
    // or the rack of one of its clips. An effect on a clip's rack is followed
    // by the whole of the layer's own stack, so that goes off with it.
    let off_from = |list: &mut Vec<EffectInstance>| {
        let mut reached = false;
        for fx in list.iter_mut() {
            reached |= fx.id == effect;
            if reached {
                fx.enabled = false;
            }
        }
        reached
    };
    if !off_from(&mut solo.effects) {
        if let lumit_core::model::LayerKind::Sequence { clips } = &mut solo.kind {
            let lt = comp
                .frame_rate
                .time_of_frame(i64::try_from(frame).ok()?)
                .ok()?
                .0
                .to_f64()
                - item.start_offset.0.to_f64();
            let mut held = false;
            for clip in clips.iter_mut() {
                if off_from(&mut clip.effects) {
                    // Another clip under the playhead is another picture:
                    // nothing of this effect's to measure there.
                    if !clip.contains(lt) {
                        return None;
                    }
                    held = true;
                }
            }
            if held {
                for fx in &mut solo.effects {
                    fx.enabled = false;
                }
            }
        }
    }
    let id = Uuid::now_v7();
    let mut scratch_comp = comp.clone();
    scratch_comp.id = id;
    scratch_comp.width = w;
    scratch_comp.height = h;
    scratch_comp.layers = vec![solo];
    let mut scratch = Document::clone(&doc);
    scratch.items.push(ProjectItem::Composition(scratch_comp));
    let scratch = Arc::new(scratch);
    crate::render::with_ready(|renderer| {
        renderer.sync_colour(&scratch);
        renderer.render_preview_linear(&scratch, id, frame, crate::render::quality_for(1.0))
    })?
    .ok()
}

/// The size of the picture `layer`'s effects run on: what the document holds
/// for a solid, a precomp or a shape, and for footage the frame the renderer
/// read when it opened the file.
fn raster_size(
    doc: &Document,
    comp: &lumit_core::model::Composition,
    layer: &lumit_core::model::Layer,
) -> (u32, u32) {
    if let lumit_core::model::LayerKind::Footage { item } = &layer.kind {
        if let Some(size) = lumit_core::fx::drivers::measure::footage_frame(*item) {
            return size;
        }
    }
    let (w, h) = lumit_core::fx::builtins::picture_size(doc, comp, layer);
    (w.round().max(1.0) as u32, h.round().max(1.0) as u32)
}

/// The measured patch for the display: a crop of the picture round the
/// sample, encoded for the screen, at most [`PATCH_SIDE`] a side.
fn patch(rgba: &[f32], w: u32, h: u32, square: [f32; 3]) -> (Vec<u8>, u32) {
    let [cx, cy, side] = square;
    let side = side.clamp(1.0, w.min(h) as f32);
    let out = PATCH_SIDE.min(side as u32).max(1);
    let x0 = (cx - side / 2.0).clamp(0.0, w as f32 - side);
    let y0 = (cy - side / 2.0).clamp(0.0, h as f32 - side);
    let mut picture = Vec::with_capacity((out * out * 4) as usize);
    for j in 0..out {
        for i in 0..out {
            let x = (x0 + (i as f32 + 0.5) * side / out as f32) as u32;
            let y = (y0 + (j as f32 + 0.5) * side / out as f32) as u32;
            let at = ((y.min(h - 1) * w + x.min(w - 1)) * 4) as usize;
            let px = rgba.get(at..at + 4).unwrap_or(&[0.0; 4]);
            let alpha = px[3];
            for c in &px[..3] {
                let straight = if alpha > 1e-6 { c / alpha } else { 0.0 };
                picture.push(
                    (lumit_core::fx::denoise::space::oetf(straight) * 255.0 + 0.5).clamp(0.0, 255.0)
                        as u8,
                );
            }
            picture.push(255);
        }
    }
    (picture, out)
}

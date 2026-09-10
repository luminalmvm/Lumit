//! Where the Audio level driver's samples come from (docs/09 §2,
//! docs/impl/node-graph.md §1.3).
//!
//! # In plain terms
//!
//! The Audio level driver turns a moment of a track into a number, so a scale
//! or a glow can follow the music. It cannot decode anything itself — it lives
//! in `lumit-core`, which knows nothing of files or codecs — so it asks a
//! **tap** for "the sound of that layer between these two moments". This module
//! is the tap, and it answers from the layer's own footage file.
//!
//! **One reading, three filters.** "That layer's sound" is answered from the
//! layer's own footage file, raw. "This comp's sound" is answered from the
//! mixer's own job list, every audible layer at its own volume with precomps
//! and solo included, summed by the mixer's own arithmetic, so the number a
//! driver reads and the sound a listener hears cannot come apart. One row of
//! that mix, and one clip of that row, are the same sum over the jobs filed
//! under them (docs/impl/audio-nodes.md §2).
//!
//! **One tap, both renders.** The preview and the export build their draw lists
//! through the same [`crate::build::build_comp_draws_at`], which makes one of
//! these from the document it was handed. There is no second implementation to
//! disagree with, which is what makes the driven picture the same in the Viewer
//! and in the file.
//!
//! **Nothing here depends on the machine.** The sound is decoded at a fixed
//! [`TAP_RATE`] rather than at whatever the sound card asked for, so two
//! computers average the *same* samples over the same window and reach the same
//! number. The playback mixer's own rate — the device's — never reaches a
//! pixel.
//!
//! **Silence is the degrade** (never a fault): a layer that is not footage, a
//! footage item that has gone, a file that is not there or will not decode, and
//! a comp in which nothing sounds all read as no sound. That is the same
//! labelled no-op a dangling matte gives.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use lumit_core::model::{Composition, Document, LayerKind, ProjectItem};
use lumit_media::AudioBuffer;
use uuid::Uuid;

/// The rate the tap decodes at, in hertz.
///
/// A **constant**, deliberately, and not the audio device's rate: the level a
/// driver reads must be a fact about the project, not about the sound card the
/// preview happens to be playing through. 48 kHz is the same rate the mixer
/// asks the decoder for on every machine that offers it.
pub const TAP_RATE: u32 = 48_000;

/// How much decoded sound the process keeps. Stereo f32 at 48 kHz is about
/// 23 MB a minute, so this is roughly ten minutes of track.
const CACHE_BUDGET_BYTES: usize = 256 * 1024 * 1024;

/// One cache entry: when it was last read, and what was decoded (`None` for a
/// file that would not decode).
type Entry = (u64, Option<Arc<AudioBuffer>>);

/// Decoded tracks by file, shared across every render in the process — the
/// preview's, the export's and the thumbnailer's alike, because what a file
/// sounds like is a fact about the file.
///
/// A failed decode is remembered as `None`, so a missing file is not reopened
/// once per driver per frame.
static DECODED: LazyLock<Mutex<HashMap<PathBuf, Entry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Ticks once per read, so entries can be ordered by how recently they were
/// touched. A counter, not a clock: two reads in the same microsecond still
/// order, and nothing here depends on the machine's time.
static TOUCH: AtomicU64 = AtomicU64::new(0);

fn entry_bytes(buffer: &Option<Arc<AudioBuffer>>) -> usize {
    buffer
        .as_ref()
        .map_or(0, |b| b.samples.len() * std::mem::size_of::<f32>())
}

/// Drop the least recently read tracks until the cache fits `budget`.
///
/// Ordering is by last read, so a comp whose two long tracks drive two
/// parameters keeps whichever it is reading and evicts the one it left behind,
/// rather than clearing everything and re-decoding both every frame.
fn evict_to_budget(cache: &mut HashMap<PathBuf, Entry>, budget: usize) {
    let mut total: usize = cache.values().map(|(_, b)| entry_bytes(b)).sum();
    if total <= budget {
        return;
    }
    let mut oldest_first: Vec<(u64, PathBuf)> = cache
        .iter()
        .map(|(path, (touched, _))| (*touched, path.clone()))
        .collect();
    oldest_first.sort_unstable();
    for (_, path) in oldest_first {
        if total <= budget {
            return;
        }
        if let Some((_, buffer)) = cache.remove(&path) {
            total -= entry_bytes(&buffer);
        }
    }
}

/// The decoded track at `path`, from the cache or by decoding it now.
///
/// ponytail: the decode happens on whichever thread asked, so the first frame
/// that reads a driven parameter waits for the whole file — the same trade the
/// media index makes, and the same one the mixer makes on its own thread. If
/// opening a project whose parameters are audio-driven holds the first frame
/// for more than a second on an ordinary track (a few minutes of stereo), hand
/// `decoded` to the decode-ahead worker so the wait happens before the frame is
/// asked for.
fn decoded(path: &Path) -> Option<Arc<AudioBuffer>> {
    // The lock is never held across the decode (14-ENGINEERING-RULES §5): FFmpeg
    // is FFI and takes as long as the file is long. Two threads racing the same
    // new file decode it twice and agree on the answer, which is cheaper than
    // holding a global lock for a second.
    if let Ok(mut cache) = DECODED.lock() {
        if let Some((touched, hit)) = cache.get_mut(path) {
            *touched = TOUCH.fetch_add(1, Ordering::Relaxed);
            return hit.clone();
        }
    }
    let decoded = lumit_media::audio::decode_all(path, TAP_RATE)
        .ok()
        .map(Arc::new);
    if let Ok(mut cache) = DECODED.lock() {
        let touched = TOUCH.fetch_add(1, Ordering::Relaxed);
        cache.insert(path.to_path_buf(), (touched, decoded.clone()));
        evict_to_budget(&mut cache, CACHE_BUDGET_BYTES);
    }
    decoded
}

/// The tap over one composition's layers.
///
/// Borrowed rather than owned: it is made where the draw list is built, from
/// the document that walk already holds, and lives exactly as long as the walk.
pub struct DocumentAudio<'a> {
    doc: &'a Arc<Document>,
    comp: &'a Composition,
    /// The comp time of the frame being built. The mix is read around *this*
    /// moment rather than around a driver's own layer time, because the mix is
    /// the composition's and the composition's clock is this one.
    t_comp: f64,
    /// Read the mix at everyone's **keyframed** Volume, ignoring any *Duck
    /// under* wires (the Out Volume socket). Only the driven-Volume
    /// evaluation itself sets this: a chain reading "this comp" would
    /// otherwise be baking the very envelope it is part of, forever.
    pre_duck: bool,
    /// The comp's audio jobs — what the mixer sums — built at most once per
    /// frame, and only when something actually asks for the mix.
    jobs: std::sync::OnceLock<Vec<crate::export::AudioJob>>,
}

impl<'a> DocumentAudio<'a> {
    /// The tap for `comp`'s layers within `doc`, for the frame at `t_comp`.
    ///
    /// A driver's Audio parameter names a layer of the composition its own
    /// layer sits in — wires never cross layers, and neither does this — or
    /// names nothing, which is the comp's own mix.
    #[must_use]
    pub fn new(doc: &'a Arc<Document>, comp: &'a Composition, t_comp: f64) -> Self {
        Self {
            doc,
            comp,
            t_comp,
            pre_duck: false,
            jobs: std::sync::OnceLock::new(),
        }
    }

    /// [`Self::new`], hearing the mix **before any duck**: what the
    /// driven-Volume evaluation reads, so one level of ducking is heard and a
    /// duck driven by a duck terminates rather than recursing.
    #[must_use]
    pub fn pre_duck(doc: &'a Arc<Document>, comp: &'a Composition, t_comp: f64) -> Self {
        Self {
            pre_duck: true,
            ..Self::new(doc, comp, t_comp)
        }
    }

    /// The file `layer` plays, if it is a footage layer whose item is still in
    /// the project.
    fn path_of(&self, layer: Uuid) -> Option<PathBuf> {
        let layer = self.comp.layers.iter().find(|l| l.id == layer)?;
        let LayerKind::Footage { item } = &layer.kind else {
            return None;
        };
        let ProjectItem::Footage(f) = self.doc.item(*item)? else {
            return None;
        };
        Some(crate::headless::footage_path(f))
    }
}

impl lumit_core::fx::AudioTap for DocumentAudio<'_> {
    /// `from` and `to` are **layer time**, which for sound is source time: a
    /// layer's start offset is exactly what the mixer subtracts to find its
    /// place in the file (`lumit_audio::mix::place_on_timeline`), so the two
    /// cannot disagree about which moment of the track a frame sits on.
    ///
    /// The half-open range `[from, to)` is taken as whole samples, clamped to
    /// the track: a window reaching before the start or past the end returns
    /// the part that exists, which is what makes the first and last frames of a
    /// clip read a real level rather than nothing.
    fn samples(&self, layer: Uuid, from: f64, to: f64, out: &mut Vec<f32>) -> Option<f64> {
        let buffer = decoded(&self.path_of(layer)?)?;
        let rate = f64::from(buffer.rate);
        if rate <= 0.0 || !from.is_finite() || !to.is_finite() {
            return None;
        }
        let frames = buffer.frames();
        let index = |t: f64| (t * rate).ceil().clamp(0.0, frames as f64) as usize;
        let (first, last) = (index(from), index(to));
        out.reserve(last.saturating_sub(first));
        for frame in first..last {
            // Mono, because a level is one number: the two channels averaged,
            // which is what the RMS of "the sound" means.
            out.push((buffer.samples[frame * 2] + buffer.samples[frame * 2 + 1]) * 0.5);
        }
        Some(rate)
    }

    /// The composition's own mix over a window centred on the frame, or the
    /// part of it that one layer, or one clip of one layer, contributes.
    ///
    /// **The filters are on the mixer's job list, not on a second reading**
    /// (docs/impl/audio-nodes.md §2). `layer` keeps the jobs filed under that
    /// mixer strip and `clip` the ones a Sequence row made from that clip; with
    /// neither of them set it is the whole mix, which is what
    /// [`lumit_core::fx::AudioTap::mix`] asks for. A job from a nested comp is
    /// filed under the outer Precomp layer, as it is for the Mixer's own
    /// strips, so a row that has become a row precomp still answers by the
    /// layer standing where it stood.
    ///
    /// **The seam is the mixer's, not a second opinion.** What layers sound,
    /// where they land, how loud they are and which of them a solo silences is
    /// [`AudioJobsBuilder`]'s answer — the same list export, playback and beat
    /// detection all mix from — and the summing is
    /// [`lumit_audio::mix::mix_stereo`] over
    /// [`place_on_timeline`](lumit_audio::mix::place_on_timeline) placements at
    /// [`crate::export::volume_bake`] gains, which is exactly
    /// `export::mix_decoded` restricted to a window. Preview and export read
    /// the same numbers because they run this same function at the same
    /// `t_comp`, with the master ceiling applied as the mixer applies it.
    ///
    /// Each clip is clipped to the window **before** its Volume is baked, so a
    /// five-minute track costs a window's arithmetic per frame rather than a
    /// track's. An animated Volume's control points therefore sit on a grid
    /// starting at the window rather than at the clip — a hair off the sound
    /// the file will carry, identical in every render of the picture, which is
    /// the property that matters here.
    ///
    /// ponytail: the layers' **audio insert chains** are deliberately not run.
    /// A driver reading the mix is asked once per picture frame, and a plugin
    /// cannot answer a window of a track thousands of times in a render — so a
    /// glow that follows the music follows the *dry* music. Bake each chain
    /// once at control rate and read the tap off that, if a plugin ever changes
    /// a level enough for the picture to notice.
    fn strip(
        &self,
        layer: Option<Uuid>,
        clip: Option<Uuid>,
        half: f64,
        out: &mut Vec<f32>,
    ) -> Option<f64> {
        if half <= 0.0 || half.is_nan() || !self.t_comp.is_finite() {
            return None;
        }
        let rate = f64::from(TAP_RATE);
        let (first, last) = (
            ((self.t_comp - half) * rate).ceil() as i64,
            ((self.t_comp + half) * rate).ceil() as i64,
        );
        let window = (last - first).max(0) as usize;
        if window == 0 {
            return None;
        }
        let jobs = self.jobs.get_or_init(|| {
            let mut jobs = crate::headless::AudioJobsBuilder::new().audio_jobs(self.doc, self.comp);
            if self.pre_duck {
                for job in &mut jobs {
                    job.driven = None;
                }
            }
            jobs
        });
        let decoded: Vec<(Arc<lumit_media::AudioBuffer>, &crate::export::AudioJob)> = jobs
            .iter()
            .filter(|job| {
                layer.is_none_or(|id| job.layer == id) && clip.is_none_or(|id| job.clip == Some(id))
            })
            .filter_map(|job| Some((decoded(&job.path)?, job)))
            .collect();
        let placed: Vec<lumit_audio::mix::PlacedAudio<'_>> = decoded
            .iter()
            .filter_map(|(buffer, job)| {
                let (start, src_start, len) = lumit_audio::mix::place_on_timeline(
                    job.in_s,
                    job.out_s,
                    job.offset_s,
                    buffer.frames(),
                    TAP_RATE,
                )?;
                // The overlap with the window, in output frames.
                let from = start.max(first);
                let to = (start + len as i64).min(last);
                if to <= from {
                    return None;
                }
                let skip = (from - start) as usize;
                let len = (to - from) as usize;
                let (gain, envelope) = crate::export::volume_bake(job, from, len, TAP_RATE);
                let head = (src_start + skip) * 2;
                Some(lumit_audio::mix::PlacedAudio {
                    start_frame: from - first,
                    samples: &buffer.samples[head..head + len * 2],
                    gain,
                    envelope,
                })
            })
            .collect();
        if placed.is_empty() {
            return None;
        }
        out.extend(lumit_audio::mix::downmix_to_mono(
            &lumit_audio::mix::mix_stereo_at(
                &placed,
                window,
                // Through the comp's master fader, because a driver reads
                // what a listener hears: pulling the master down
                // must dim a glow that follows the music, not only the sound.
                lumit_audio::mix::db_to_gain(self.comp.master_volume_db),
            ),
        ));
        Some(rate)
    }
}

/// A fingerprint of what `comp`'s mix sounds like, for the frame key
/// (docs/impl/audio-nodes.md §3).
///
/// A frame drawn through a driver reading *This comp* depends on the mix, and
/// the mix is not in the picture's name: a Volume is a mixer control, so
/// [`lumit_eval`] hashes no part of it and pulling a fader would hand back the
/// frame drawn before the pull. This is the missing term: the same job list
/// [`DocumentAudio::strip`] reads, folded down to eight bytes.
///
/// **What the reading uses, and nothing else**: each job's file, strip, clip
/// and placement, the gains [`crate::export::volume_bake`] bakes (Volume, Pan,
/// the carriers, a clip's fade and a *Duck under* wire) and the master fader.
/// The layers' insert chains are left out because the reading does not run
/// them (see [`DocumentAudio::strip`]), so a plugin knob that cannot change
/// the number must not retire a frame.
///
/// ponytail: this builds the comp's job list a second time, next door to the
/// tap's own. It runs once per frame key and only for a comp that actually
/// reads its mix; memoise it on the walk if a project full of them ever shows
/// up in a key-building profile.
#[must_use]
pub fn mix_fingerprint(doc: &Arc<Document>, comp: &Composition) -> u64 {
    let jobs = crate::headless::AudioJobsBuilder::new().audio_jobs(doc, comp);
    let mut h = blake3::Hasher::new();
    h.update(&comp.master_volume_db.to_bits().to_le_bytes());
    for job in &jobs {
        h.update(job.path.to_string_lossy().as_bytes());
        h.update(job.layer.as_bytes());
        if let Some(clip) = job.clip {
            h.update(clip.as_bytes());
        }
        for v in [job.in_s, job.out_s, job.offset_s] {
            h.update(&v.to_bits().to_le_bytes());
        }
        // Debug text rather than a field-by-field walk, as the bridge's
        // `jobs_signature` folds a graph and a fade: a job carries a handful of
        // each, and this is a hash rather than a document.
        h.update(
            format!(
                "{:?}",
                (
                    &job.volume,
                    &job.pan,
                    &job.fade,
                    job.carriers
                        .iter()
                        .map(|c| (&c.volume, &c.pan, c.offset_s))
                        .collect::<Vec<_>>(),
                    job.driven.as_ref().map(|d| format!("{:?}", d.graph)),
                )
            )
            .as_bytes(),
        );
    }
    let mut first = [0u8; 8];
    first.copy_from_slice(&h.finalize().as_bytes()[..8]);
    u64::from_le_bytes(first)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use lumit_core::fx::AudioTap;

    /// A document with one footage layer pointing at `path`: the document, its
    /// composition's id, and the layer's id.
    fn doc_with_audio_layer(path: &Path) -> (Arc<Document>, Uuid, Uuid) {
        let (doc, comp, layers) = doc_with_audio_layers(path, 1);
        let layer = layers[0];
        (Arc::new(doc), comp, layer)
    }

    /// The same document with `rows` layers on it, all playing `path`, and the
    /// document left unwrapped so a test can set a Volume on a row before it
    /// reads the mix.
    fn doc_with_audio_layers(path: &Path, rows: usize) -> (Document, Uuid, Vec<Uuid>) {
        use lumit_core::model::{FootageItem, Layer, LinearColour, MediaRef, Switches};
        use lumit_core::time::{CompTime, Duration, FrameRate, Rational};

        let mut doc = Document::new();
        let item = Uuid::now_v7();
        doc.items.push(ProjectItem::Footage(FootageItem {
            sequence: None,
            id: item,
            name: "tone.flac".into(),
            media: MediaRef {
                relative_path: "tone.flac".into(),
                absolute_path: path.to_string_lossy().into_owned(),
                fingerprint: None,
                extra: serde_json::Map::new(),
            },
            extra: serde_json::Map::new(),
            colour_space: None,
        }));
        let layer = |n: usize| Layer {
            id: Uuid::now_v7(),
            name: format!("Tone {n}"),
            kind: LayerKind::Footage { item },
            in_point: CompTime(Rational::new(0, 1).unwrap()),
            out_point: CompTime(Rational::new(1, 1).unwrap()),
            start_offset: CompTime(Rational::new(0, 1).unwrap()),
            transform: Default::default(),
            graph: Default::default(),
            markers: Vec::new(),
            matte: None,
            parent: None,
            label: 0,
            volume_db: lumit_core::anim::Property::zero(),
            pan: lumit_core::anim::Property::zero(),
            audio_only: true,
            adjustment: false,
            retime: None,
            interpolation: Default::default(),
            parked_flow: None,
            blend: Default::default(),
            masks: Vec::new(),
            paint: Vec::new(),
            puppet: None,
            effects: Vec::new(),
            styles: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        };
        let layers: Vec<Layer> = (0..rows).map(layer).collect();
        let layer_ids = layers.iter().map(|l| l.id).collect();
        let comp_id = Uuid::now_v7();
        doc.items.push(ProjectItem::Composition(Composition {
            master_volume_db: 0.0,
            sound_mix: false,
            groups: Vec::new(),
            beat_grid: None,
            id: comp_id,
            name: "Scene".into(),
            width: 32,
            height: 16,
            frame_rate: FrameRate::new(30, 1).unwrap(),
            duration: Duration(Rational::new(5, 1).unwrap()),
            background: LinearColour::BLACK,
            work_area: None,
            layers,
            markers: Vec::new(),
            motion_blur: Default::default(),
            extra: serde_json::Map::new(),
        }));
        (doc, comp_id, layer_ids)
    }

    /// The tone fixture, or `None` on a machine with no FFmpeg CLI to make it.
    fn tone(dir: &Path) -> Option<PathBuf> {
        let path = lumit_media::index::tests_support::tone(dir);
        if path.is_none() {
            eprintln!("no ffmpeg CLI: the tone row is skipped");
        }
        path
    }

    /// The loudest sample in a run.
    fn loudest(samples: &[f32]) -> f32 {
        samples.iter().fold(0.0f32, |top, s| top.max(s.abs()))
    }

    /// Two tracks under a budget with room for one: the one just read stays,
    /// the one left behind goes. A comp driving two parameters from two long
    /// tracks therefore alternates rather than re-decoding both every frame.
    #[test]
    fn the_least_recently_read_track_is_the_one_evicted() {
        let track = |seconds: usize| {
            Some(Arc::new(AudioBuffer {
                rate: TAP_RATE,
                samples: vec![0.0; seconds * TAP_RATE as usize * 2],
            }))
        };
        let one_track = entry_bytes(&track(1));
        let budget = one_track + one_track / 2;

        let mut cache: HashMap<PathBuf, Entry> = HashMap::new();
        cache.insert(PathBuf::from("a.wav"), (0, track(1)));
        cache.insert(PathBuf::from("b.wav"), (1, track(1)));
        evict_to_budget(&mut cache, budget);
        assert_eq!(
            cache.keys().collect::<Vec<_>>(),
            vec![&PathBuf::from("b.wav")],
            "the track read last is the one kept"
        );

        // Reading a again makes b the older of the two, and the next eviction
        // reverses — which is the alternation, not the thrash.
        cache.insert(PathBuf::from("a.wav"), (2, track(1)));
        evict_to_budget(&mut cache, budget);
        assert_eq!(
            cache.keys().collect::<Vec<_>>(),
            vec![&PathBuf::from("a.wav")],
            "and the one left behind is the one dropped"
        );

        // Under budget, nothing is touched.
        evict_to_budget(&mut cache, budget);
        assert_eq!(cache.len(), 1, "a cache that fits is left alone");
    }

    /// A layer that is not footage, or footage whose file is not there, reads
    /// as silence rather than failing — the documented degrade.
    #[test]
    fn a_layer_with_no_sound_reads_as_silence() {
        let dir = tempfile::tempdir().expect("temp dir");
        let missing = dir.path().join("not-here.wav");
        let (doc, comp_id, layer) = doc_with_audio_layer(&missing);
        let comp = doc.comp(comp_id).expect("comp");
        let tap = DocumentAudio::new(&doc, comp, 0.0);

        let mut out = Vec::new();
        assert_eq!(
            tap.samples(layer, 0.0, 0.1, &mut out),
            None,
            "a file that is not there is silence, not a fault"
        );
        assert_eq!(
            tap.samples(Uuid::now_v7(), 0.0, 0.1, &mut out),
            None,
            "and so is a reference naming no layer at all"
        );
        assert!(out.is_empty());
    }

    /// The tap is a pure function of the file, the layer and the window: two
    /// reads of the same moment give the same samples, which is what makes the
    /// preview and the export agree on the number.
    #[test]
    fn the_same_window_reads_the_same_samples_twice() {
        let dir = tempfile::tempdir().expect("temp dir");
        let Some(path) = tone(dir.path()) else {
            return;
        };
        let (doc, comp_id, layer) = doc_with_audio_layer(&path);
        let comp = doc.comp(comp_id).expect("comp");
        let tap = DocumentAudio::new(&doc, comp, 0.0);

        let (mut first, mut second) = (Vec::new(), Vec::new());
        let rate = tap.samples(layer, 0.4, 0.45, &mut first).expect("a rate");
        assert_eq!(
            rate,
            f64::from(TAP_RATE),
            "the tap's own rate, not a device's"
        );
        assert_eq!(tap.samples(layer, 0.4, 0.45, &mut second), Some(rate));
        assert_eq!(first, second, "the same window reads the same samples");
        assert!(!first.is_empty(), "a tone is not silence");
        assert!(
            first.iter().any(|s| s.abs() > 0.01),
            "and the samples are the sound, not zeroes"
        );

        // A window reaching before the track's start returns the part that
        // exists rather than nothing.
        let mut early = Vec::new();
        assert!(tap.samples(layer, -0.1, 0.02, &mut early).is_some());
        assert_eq!(
            early.len(),
            (0.02 * f64::from(TAP_RATE)).ceil() as usize,
            "the window is clamped to the track, not refused"
        );
    }

    /// **One reading, three filters** (docs/impl/audio-nodes.md §2, plan 1).
    ///
    /// `strip` with neither filter set is the mix `mix` gives; filtered to a
    /// row it is that row alone **at the row's own Volume**, which is the whole
    /// difference between this and the raw `samples` read; and the rows sum
    /// back to the mix, because a filter is a filter and not a second
    /// arithmetic.
    #[test]
    fn strip_reads_the_whole_mix_and_one_row_of_it_post_fader() {
        let dir = tempfile::tempdir().expect("temp dir");
        let Some(path) = tone(dir.path()) else {
            return;
        };
        let (mut doc, comp_id, rows) = doc_with_audio_layers(&path, 2);
        // The second row half as loud, so "post-fader" is a number and not a
        // claim: half is exactly −6.02 dB.
        let half = 20.0 * 0.5f64.log10();
        doc.comp_mut(comp_id).expect("comp").layers[1].volume_db =
            lumit_core::anim::Property::fixed(half);
        let doc = Arc::new(doc);
        let comp = doc.comp(comp_id).expect("comp");
        let tap = DocumentAudio::new(&doc, comp, 0.5);

        let read = |layer, clip| {
            let mut out = Vec::new();
            let rate = tap.strip(layer, clip, 0.05, &mut out);
            (rate, out)
        };
        let (rate, whole) = read(None, None);
        assert_eq!(rate, Some(f64::from(TAP_RATE)));
        assert!(loudest(&whole) > 0.01, "the two tones are heard");

        let mut mixed = Vec::new();
        assert_eq!(tap.mix(0.05, &mut mixed), rate);
        assert_eq!(mixed, whole, "mix is strip with neither filter set");

        let (loud_rate, loud) = read(Some(rows[0]), None);
        let (_, quiet) = read(Some(rows[1]), None);
        assert_eq!(loud_rate, rate);
        assert_eq!(loud.len(), whole.len(), "one row, the same window");
        assert!(loudest(&loud) > 0.01, "a row on its own is not silence");
        assert!(
            (loudest(&quiet) - loudest(&loud) * 0.5).abs() < 0.01,
            "the fader is heard: {} against {}",
            loudest(&quiet),
            loudest(&loud)
        );
        for (n, ((a, b), m)) in loud.iter().zip(&quiet).zip(&whole).enumerate() {
            assert!(
                (a + b - m).abs() < 1e-5,
                "sample {n}: the rows must sum to the mix, {a} + {b} against {m}"
            );
        }

        // A row nobody has is silence, not a fault: the documented degrade.
        let (missing, out) = read(Some(Uuid::now_v7()), None);
        assert_eq!(missing, None);
        assert!(out.is_empty());
    }

    /// **A clip is the same reading filtered again** (plan 1): the clips of a
    /// row sum back to the row, a clip that is not playing in the window is
    /// silence, and the one asked for carries its own fade.
    #[test]
    fn strip_reads_one_clip_of_a_row_with_its_fade() {
        use lumit_core::sequence::{Clip, ClipSource, Fade};
        use lumit_core::time::{CompTime, Rational};

        let dir = tempfile::tempdir().expect("temp dir");
        let Some(path) = tone(dir.path()) else {
            return;
        };
        let (mut doc, comp_id, rows) = doc_with_audio_layers(&path, 1);
        let comp = doc.comp_mut(comp_id).expect("comp");
        let LayerKind::Footage { item } = comp.layers[0].kind else {
            panic!("the fixture is a footage row");
        };
        let second = Rational::new(1, 1).expect("a second");
        let clip = |at: i64| {
            Clip::new(
                ClipSource::Footage(item),
                Rational::ZERO,
                second,
                Rational::new(at, 1).expect("a whole second"),
                second,
            )
        };
        // Two clips butt-cut at a second, the first rising out of silence over
        // its own first half second.
        let mut head = clip(0);
        head.fade_in = Fade {
            seconds: Rational::new(1, 2).expect("half a second"),
            ..Fade::default()
        };
        let (head_id, tail_id) = (head.id, Uuid::now_v7());
        let mut tail = clip(1);
        tail.id = tail_id;
        comp.layers[0].kind = LayerKind::Sequence {
            clips: vec![head, tail],
        };
        comp.layers[0].out_point = CompTime(Rational::new(2, 1).expect("two seconds"));
        let doc = Arc::new(doc);
        let comp = doc.comp(comp_id).expect("comp");

        // Over the join, both clips play and the two sum to the row.
        let tap = DocumentAudio::new(&doc, comp, 1.0);
        let read = |tap: &DocumentAudio<'_>, clip| {
            let mut out = Vec::new();
            let rate = tap.strip(Some(rows[0]), clip, 0.25, &mut out);
            (rate, out)
        };
        let (rate, row) = read(&tap, None);
        assert_eq!(rate, Some(f64::from(TAP_RATE)));
        let (_, first) = read(&tap, Some(head_id));
        let (_, last) = read(&tap, Some(tail_id));
        assert!(loudest(&first) > 0.01 && loudest(&last) > 0.01, "both play");
        for (n, ((a, b), m)) in first.iter().zip(&last).zip(&row).enumerate() {
            assert!(
                (a + b - m).abs() < 1e-5,
                "sample {n}: the clips must sum to the row, {a} + {b} against {m}"
            );
        }

        // Inside the head clip's fade, and before the tail starts: the ramp is
        // heard, and the clip that is not playing is silence rather than a
        // fault.
        let early = DocumentAudio::new(&doc, comp, 0.25);
        let (_, ramp) = read(&early, Some(head_id));
        let (quiet_rate, quiet) = read(&early, Some(tail_id));
        assert_eq!(quiet_rate, None, "a clip outside the window is silence");
        assert!(quiet.is_empty());
        let mid = ramp.len() / 2;
        assert!(
            loudest(&ramp[..mid]) < loudest(&ramp[mid..]) * 0.75,
            "the clip's own fade is in what the reading gives: {} against {}",
            loudest(&ramp[..mid]),
            loudest(&ramp[mid..])
        );
    }

    /// **The mix is in the frame's name** (docs/impl/audio-nodes.md §3, plan
    /// 5). A fader the picture follows changes the fingerprint the key folds;
    /// a name, which no listener hears, leaves it exactly where it was.
    #[test]
    fn the_mix_fingerprint_moves_with_a_fader_and_not_with_a_name() {
        let dir = tempfile::tempdir().expect("temp dir");
        let Some(path) = tone(dir.path()) else {
            return;
        };
        let (mut doc, comp_id, _) = doc_with_audio_layers(&path, 2);
        let sig = |doc: &Document| {
            let doc = Arc::new(doc.clone());
            let comp = doc.comp(comp_id).expect("comp").clone();
            mix_fingerprint(&doc, &comp)
        };

        let before = sig(&doc);
        doc.comp_mut(comp_id).expect("comp").layers[0].name = "Renamed".into();
        assert_eq!(sig(&doc), before, "a name is not a sound");

        doc.comp_mut(comp_id).expect("comp").layers[0].volume_db =
            lumit_core::anim::Property::fixed(-6.0);
        let pulled = sig(&doc);
        assert_ne!(
            pulled, before,
            "a fader moves the mix, so it must move the frame's name"
        );

        doc.comp_mut(comp_id).expect("comp").master_volume_db = -3.0;
        assert_ne!(sig(&doc), pulled, "and so does the master");
    }
}

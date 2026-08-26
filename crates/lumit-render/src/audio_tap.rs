//! Where the Audio level driver's samples come from (K-471 §1.3, docs/09 §2,
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
//! **One tap, both renders.** The preview and the export build their draw lists
//! through the same [`crate::build::build_comp_draws_at`], which makes one of
//! these from the document it was handed. There is no second implementation to
//! disagree with, which is what makes the driven picture the same in the Viewer
//! and in the file (K-031).
//!
//! **Nothing here depends on the machine.** The sound is decoded at a fixed
//! [`TAP_RATE`] rather than at whatever the sound card asked for, so two
//! computers average the *same* samples over the same window and reach the same
//! number. The playback mixer's own rate — the device's — never reaches a
//! pixel.
//!
//! **Silence is the degrade** (never a fault): a layer that is not footage, a
//! footage item that has gone, a file that is not there or will not decode, and
//! a reference that names no layer at all all read as no sound. That is the
//! same labelled no-op a dangling matte gives.

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
    doc: &'a Document,
    comp: &'a Composition,
}

impl<'a> DocumentAudio<'a> {
    /// The tap for `comp`'s layers within `doc`.
    ///
    /// A driver's Audio parameter names a layer of the composition its own
    /// layer sits in — wires never cross layers, and neither does this
    /// (K-471 §1.3).
    #[must_use]
    pub fn new(doc: &'a Document, comp: &'a Composition) -> Self {
        Self { doc, comp }
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
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use lumit_core::fx::AudioTap;

    /// A document with one footage layer pointing at `path`: the document, its
    /// composition's id, and the layer's id.
    fn doc_with_audio_layer(path: &Path) -> (Document, Uuid, Uuid) {
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
        let layer = Layer {
            id: Uuid::now_v7(),
            name: "Tone".into(),
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
            audio_only: true,
            adjustment: false,
            retime: None,
            interpolation: Default::default(),
            parked_flow: None,
            blend: Default::default(),
            masks: Vec::new(),
            paint: Vec::new(),
            effects: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        };
        let layer_id = layer.id;
        let comp_id = Uuid::now_v7();
        doc.items.push(ProjectItem::Composition(Composition {
            id: comp_id,
            name: "Scene".into(),
            width: 32,
            height: 16,
            frame_rate: FrameRate::new(30, 1).unwrap(),
            duration: Duration(Rational::new(5, 1).unwrap()),
            background: LinearColour::BLACK,
            work_area: None,
            layers: vec![layer],
            markers: Vec::new(),
            motion_blur: Default::default(),
            extra: serde_json::Map::new(),
        }));
        (doc, comp_id, layer_id)
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
        let tap = DocumentAudio::new(&doc, comp);

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
    /// preview and the export agree on the number (K-031).
    #[test]
    fn the_same_window_reads_the_same_samples_twice() {
        let dir = tempfile::tempdir().expect("temp dir");
        let Some(path) = lumit_media::index::tests_support::tone(dir.path()) else {
            eprintln!("no ffmpeg CLI: the tone row is skipped");
            return;
        };
        let (doc, comp_id, layer) = doc_with_audio_layer(&path);
        let comp = doc.comp(comp_id).expect("comp");
        let tap = DocumentAudio::new(&doc, comp);

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
}

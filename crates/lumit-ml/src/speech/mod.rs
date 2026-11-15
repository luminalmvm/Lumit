//! Speech to text: the words somebody said, when they said each one, and who
//! was speaking, written down by Whisper on this machine (docs/impl/addons.md
//! §4, the speech task).
//!
//! # In plain terms
//!
//! Audio is fed in at 16 kHz as it is decoded. A small model listens for
//! speech as it arrives, and silence is never shown to Whisper, which is the
//! main thing that stops it inventing sentences out of nothing. The speech is
//! gathered into windows of at most thirty seconds, each window is written
//! down on its own, and every word's time is worked out from where the model
//! looked while writing it and then nudged to the nearest quiet point. Whisper
//! leaves out "um" and "uh", so wherever the listener heard a voice and no
//! word covers it, an "um" is put in with a confidence of exactly zero: no
//! word the model wrote ever has that. A pause of a second starts a
//! paragraph. When asked, a third model fingerprints the voices and the
//! fingerprints are grouped into speakers, and [`cluster`] does the same
//! grouping across several recordings so one person keeps one number.
//!
//! # Thread role and contract
//!
//! A [`Speech`] owns every session it opens and is never shared: each run is
//! an FFI call that may hold the GPU (14-ENGINEERING-RULES §1.3). It lives on
//! the analysis job's own thread. A [`Run`] does a bounded amount of work per
//! [`Run::feed`]: the listener over the samples handed in, and a window
//! written down whenever one fills, so a caller feeding a few seconds at a
//! time can cancel between calls by dropping the run. Models are not
//! bit-stable, so what comes back is a baked analysis kept beside the media,
//! never something worked out inside a render (§3).

mod align;
mod config;
mod decode;
mod fbank;
mod languages;
mod mel;
mod run;
mod tokens;
mod vad;
mod voices;
mod whisper;

use serde::{Deserialize, Serialize};

use crate::{
    error::MlError,
    manifest::{Arch, Task},
    runtime,
    session::Prefer,
    store,
};

/// What to write down, and how.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Options {
    /// The language spoken, by code, or `None` to let the model tell from
    /// the first window of speech.
    pub language: Option<String>,
    /// Write the words in English whatever language they were spoken in.
    pub translate: bool,
    /// Whether and how to tell voices apart.
    pub speakers: Speakers,
    /// Words and names the speakers use, as the user typed them, separated
    /// by commas or new lines. Empty for none. Shown to the model ahead of
    /// every window, which is how Whisper learns a spelling.
    pub vocabulary: String,
}

/// Whether and how to tell voices apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Speakers {
    /// Nobody is told apart.
    #[default]
    Off,
    /// As many speakers as the voices sound like.
    Detect,
    /// Exactly this many.
    Count(u32),
}

/// The installed speech pack, open.
#[derive(Debug)]
pub struct Speech {
    whisper: whisper::Whisper,
    ear: vad::Ear,
    voices: Option<voices::Voices>,
    identity: [u8; 32],
    model: String,
    translates: bool,
}

/// One transcription in progress.
#[derive(Debug)]
pub struct Run<'s> {
    speech: &'s mut Speech,
    state: run::State,
}

/// Everything heard in one recording.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Heard {
    /// The language spoken, by code, or empty when nothing was heard.
    pub language: String,
    /// Every word, in order, never overlapping.
    pub words: Vec<HeardWord>,
    /// Each stretch of one voice. Empty when speakers were not asked for.
    pub voices: Vec<Voice>,
}

/// One word.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeardWord {
    /// The word as written, punctuation included, without spaces round it.
    pub text: String,
    /// Seconds into the audio fed.
    pub start: f64,
    /// Seconds into the audio fed.
    pub end: f64,
    /// The mean probability of its word pieces, at least 0.01. Exactly 0.0
    /// for a filler put in where a voice was heard and nothing was written.
    pub confidence: f32,
    /// Which of [`Heard::voices`] said it.
    pub voice: Option<usize>,
    /// Whether it starts a paragraph.
    pub paragraph: bool,
}

/// One stretch of one voice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Voice {
    /// Seconds into the audio fed.
    pub start: f64,
    /// Seconds into the audio fed.
    pub end: f64,
    /// Its fingerprint, of length one.
    pub embedding: Vec<f32>,
}

/// The installed speech pack a transcription uses: the first, or with
/// `translate`, the first that can write what was said in English, when one
/// is installed.
#[must_use]
pub fn pick(translate: bool) -> Option<store::Installed> {
    let packs = store::snapshot();
    let speech = || {
        packs
            .iter()
            .filter(|p| !p.broken && p.manifest.task() == Some(Task::Speech))
    };
    let translates = |p: &&store::Installed| matches!(p.manifest.model.as_ref().map(|m| &m.arch), Some(Arch::Whisper(w)) if w.translates);
    translate
        .then(|| speech().find(translates))
        .flatten()
        .or_else(|| speech().next())
        .cloned()
}

impl Speech {
    /// Open the speech pack [`pick`] names, on the platform's accelerator
    /// first.
    ///
    /// # Errors
    ///
    /// [`MlError::RuntimeMissing`] when no model runtime is installed,
    /// [`MlError::PackMissing`] when no speech pack is, and whatever
    /// [`crate::session::Session::open`] answers for a pack that is there and
    /// will not open.
    pub fn open(translate: bool) -> Result<Self, MlError> {
        Self::open_on(Prefer::Platform, translate)
    }

    /// [`Speech::open`], on the provider asked for.
    fn open_on(prefer: Prefer, translate: bool) -> Result<Self, MlError> {
        if matches!(runtime::status(), runtime::RuntimeStatus::Missing) {
            return Err(MlError::RuntimeMissing);
        }
        let installed = pick(translate).ok_or(MlError::PackMissing(Task::Speech))?;
        let Some(Arch::Whisper(names)) = installed.manifest.model.as_ref().map(|model| &model.arch)
        else {
            return Err(MlError::PackUnreadable);
        };
        let whisper = whisper::Whisper::open(&installed.path, names, prefer)?;
        let ear = vad::Ear::open(&installed.path, names)?;
        let voices = voices::Voices::open(&installed.path, names)?;
        Ok(Speech {
            whisper,
            ear,
            voices,
            identity: store::identity(&installed.manifest),
            model: format!("{} {}", installed.manifest.id, installed.manifest.version),
            translates: names.translates,
        })
    }

    /// What this pack is, for the key a transcription is filed under (§7).
    #[must_use]
    pub fn identity(&self) -> [u8; 32] {
        self.identity
    }

    /// Which provider took Whisper.
    #[must_use]
    pub fn provider(&self) -> &'static str {
        self.whisper.provider()
    }

    /// The pack's id and version.
    #[must_use]
    pub fn model(&self) -> String {
        self.model.clone()
    }

    /// Whether the pack carries a speaker model.
    #[must_use]
    pub fn tells_speakers(&self) -> bool {
        self.voices.is_some()
    }

    /// Whether the pack writes other languages in English well, so that
    /// [`Options::translate`] is worth offering.
    #[must_use]
    pub fn translates(&self) -> bool {
        self.translates
    }

    /// Start writing down one recording.
    ///
    /// # Errors
    ///
    /// [`MlError::Invalid`] for a language this model does not know.
    pub fn start(&mut self, options: &Options) -> Result<Run<'_>, MlError> {
        if let Some(code) = &options.language {
            if self.whisper.settings.language(code).is_none() {
                return Err(MlError::Invalid(format!(
                    "this model does not know the language \"{code}\""
                )));
            }
        }
        self.ear.reset();
        let terms: Vec<&str> = options
            .vocabulary
            .split([',', '\n'])
            .map(str::trim)
            .filter(|term| !term.is_empty())
            .collect();
        let vocabulary = if terms.is_empty() {
            Vec::new()
        } else {
            self.whisper
                .tokens
                .encode(&format!(" {}.", terms.join(", ")))
        };
        Ok(Run {
            state: run::State::new(options.clone(), vocabulary),
            speech: self,
        })
    }
}

/// Group the voices of several recordings into speakers, so the same person
/// gets the same number in each: per recording, per voice, a speaker number
/// shared across all of them.
///
/// With a `count`, exactly that many speakers (fewer only when there are
/// fewer voices); without one, as many as the voices sound like.
#[must_use]
pub fn cluster(items: &[&[Voice]], count: Option<u32>) -> Vec<Vec<u32>> {
    let prints: Vec<&[f32]> = items
        .iter()
        .flat_map(|item| item.iter().map(|voice| voice.embedding.as_slice()))
        .collect();
    let numbers = voices::group(&prints, count.map(|c| c as usize), run::BETWEEN_STRETCHES);
    let mut at = 0;
    items
        .iter()
        .map(|item| {
            let out = numbers.get(at..at + item.len()).unwrap_or(&[]).to_vec();
            at += item.len();
            out
        })
        .collect()
}

/// Every language Whisper knows: code and the language's own name for
/// itself, in Whisper's order.
#[must_use]
pub fn languages() -> &'static [(&'static str, &'static str)] {
    &languages::LANGUAGES
}

/// Whether a working speech pack and a runtime to run it are installed.
#[must_use]
pub fn installed() -> bool {
    matches!(
        runtime::status(),
        runtime::RuntimeStatus::Present | runtime::RuntimeStatus::Loaded { .. }
    ) && store::find(Task::Speech).is_some()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::path::Path;

    /// 16-bit PCM samples from a WAV file, as -1..1.
    fn wav(path: &Path) -> Vec<f32> {
        let bytes = std::fs::read(path).unwrap();
        let mut at = 12;
        while at + 8 <= bytes.len() {
            let len = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
            if &bytes[at..at + 4] == b"data" {
                return bytes[at + 8..(at + 8 + len).min(bytes.len())]
                    .chunks_exact(2)
                    .map(|pair| f32::from(i16::from_le_bytes([pair[0], pair[1]])) / 32768.0)
                    .collect();
            }
            at += 8 + len + (len & 1);
        }
        panic!("{} has no samples", path.display());
    }

    /// Lower case letters and apostrophes, for comparing words.
    fn plain(word: &str) -> String {
        word.to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '\'')
            .collect()
    }

    /// **On the reference machine, a real pack writes down a dialogue between
    /// three voices: nearly every word, in order, never overlapping, and told
    /// apart into more than one speaker.**
    ///
    /// Reads `whisper-small/`, an installed pack folder, and `dialogue.wav`
    /// with `dialogue.txt` beside it, from `LUMIT_ML_PACKS_DIR`. What the
    /// model writes is its own business, so only a word error rate under one
    /// in five is held to, and the contract the bridge relies on: sorted,
    /// non-overlapping words inside the recording, a real word's confidence
    /// never exactly zero, and every voice index pointing somewhere. Skipped
    /// politely wherever the runtime and the pack are not installed, which is
    /// every CI runner.
    #[test]
    fn the_real_model_writes_down_a_dialogue() {
        let (Some(runtime_dir), Some(packs)) = (
            crate::test_support::runtime_dir(),
            crate::test_support::packs_dir(),
        ) else {
            runtime::no_runtime();
            return;
        };
        let pack = packs.join("whisper-small");
        let (audio, expected) = (packs.join("dialogue.wav"), packs.join("dialogue.txt"));
        if !pack.join(store::MANIFEST_FILE).is_file() || !audio.is_file() {
            runtime::no_runtime();
            return;
        }
        runtime::load(&runtime_dir).expect("the runtime would not load");

        let _serial = crate::test_support::serially();
        // A pack of its own, so nothing here reads or writes the user's.
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("whisper-small");
        std::fs::create_dir_all(&home).unwrap();
        for entry in std::fs::read_dir(&pack).unwrap() {
            let from = entry.unwrap().path();
            let to = home.join(from.file_name().unwrap());
            if std::fs::hard_link(&from, &to).is_err() {
                std::fs::copy(&from, &to).unwrap();
            }
        }
        store::with_dir(Some(root.path().to_path_buf()));

        let mut speech = Speech::open(false).expect("the pack would not open");
        assert!(speech.tells_speakers(), "the pack carries a speaker model");
        let samples = wav(&audio);
        let length = samples.len() as f64 / 16_000.0;
        let mut run = speech
            .start(&Options {
                speakers: Speakers::Detect,
                ..Options::default()
            })
            .unwrap();
        for block in samples.chunks(16_000) {
            run.feed(block).unwrap();
        }
        let heard = run.finish().unwrap();
        store::with_dir(None);

        assert_eq!(heard.language, "en");
        let mut last_end = 0.0;
        for word in &heard.words {
            assert!(
                word.start >= last_end - 1e-9,
                "{word:?} overlaps the word before"
            );
            assert!(
                word.end >= word.start && word.end <= length + 0.5,
                "{word:?}"
            );
            assert!(
                word.confidence == 0.0 || word.confidence >= 0.01,
                "{word:?}"
            );
            if let Some(voice) = word.voice {
                assert!(voice < heard.voices.len(), "{word:?} names no voice");
            }
            last_end = word.end;
        }

        // Word error rate against the script, fillers left out.
        let wanted: Vec<String> = std::fs::read_to_string(expected)
            .unwrap()
            .split_whitespace()
            .map(plain)
            .collect();
        let got: Vec<String> = heard
            .words
            .iter()
            .filter(|word| word.confidence > 0.0)
            .map(|word| plain(&word.text))
            .filter(|word| !word.is_empty())
            .collect();
        let mut row: Vec<usize> = (0..=got.len()).collect();
        for (i, want) in wanted.iter().enumerate() {
            let mut next = vec![i + 1; got.len() + 1];
            for (j, have) in got.iter().enumerate() {
                next[j + 1] = (row[j] + usize::from(want != have))
                    .min(row[j + 1] + 1)
                    .min(next[j] + 1);
            }
            row = next;
        }
        let rate = row[got.len()] as f64 / wanted.len() as f64;
        assert!(rate < 0.2, "word error rate {rate:.2}: {got:?}");

        let numbers = cluster(&[&heard.voices], None);
        let speakers = numbers[0].iter().copied().max().map_or(0, |top| top + 1);
        assert!(
            speakers >= 2,
            "three voices came back as {speakers} speaker"
        );
    }
}

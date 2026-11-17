//! The work itself, on the `lumit-speech` thread.

use super::*;

/// One file to hear, read off the snapshot before any work starts.
pub(super) struct Plan {
    pub(super) item: Uuid,
    // Only read where the build can decode sound.
    #[cfg_attr(not(feature = "media"), allow(dead_code))]
    pub(super) path: PathBuf,
    pub(super) stream: u32,
    pub(super) seconds: f64,
    pub(super) cache: Option<PathBuf>,
}

pub(super) fn run(
    project: Uuid,
    items: &[Uuid],
    options: &BridgeTranscribeOptions,
    cancel: &AtomicBool,
    say: &mut dyn FnMut(BridgeTranscribeEvent),
) -> Result<BridgeTranscribeOutcome, BridgeError> {
    let mut outcome = BridgeTranscribeOutcome {
        done: 0,
        failed: 0,
        cancelled: false,
    };
    let event =
        |item: Uuid, fraction: f64, stage, speed: f64, remaining: f64| BridgeTranscribeEvent {
            item: item.to_string(),
            fraction: fraction.clamp(0.0, 1.0),
            stage,
            speed,
            remaining,
        };
    let first = items.first().copied().unwrap_or_else(Uuid::nil);
    say(event(first, 0.0, BridgeTranscribeStage::Loading, 0.0, 0.0));
    let plans = plan(project, items, options);
    // The model loads on the first file nobody has heard before, so asking
    // again for words already heard is as quick as reading them.
    let pack = lumit_ml::speech::pick(options.translate);
    let whisper = pack
        .as_ref()
        .and_then(|p| p.manifest.model.as_ref())
        .and_then(|m| match &m.arch {
            lumit_ml::manifest::Arch::Whisper(w) => Some(w),
            _ => None,
        });
    let (Some(pack), Some(whisper)) = (pack.as_ref(), whisper) else {
        say(event(first, 1.0, BridgeTranscribeStage::Failed, 0.0, 0.0));
        return Err(BridgeError::AddonMissing);
    };
    let model = format!("{} {}", pack.manifest.id, pack.manifest.version);
    let tells_speakers = whisper.voices.is_some();
    let mut speech: Option<lumit_ml::speech::Speech> = None;
    let heard_options = lumit_ml::speech::Options {
        language: Some(options.language.trim().to_owned()).filter(|l| !l.is_empty()),
        translate: options.translate,
        vocabulary: options.vocabulary.trim().to_owned(),
        speakers: match options.speakers {
            0 if tells_speakers => lumit_ml::speech::Speakers::Detect,
            n if n >= 2 && tells_speakers => lumit_ml::speech::Speakers::Count(n),
            _ => lumit_ml::speech::Speakers::Off,
        },
    };
    let total: f64 = plans
        .iter()
        .map(|p| p.as_ref().map_or(0.0, |p| p.seconds))
        .sum::<f64>()
        .max(1e-6);
    let started = web_time::Instant::now();
    let mut behind = 0.0;
    let mut voices = super::voices::Voices::default();
    for (index, planned) in plans.into_iter().enumerate() {
        let item = items.get(index).copied().unwrap_or_else(Uuid::nil);
        if cancel.load(Ordering::Relaxed) {
            outcome.cancelled = true;
            break;
        }
        let Some(plan) = planned else {
            outcome.failed += 1;
            say(event(
                item,
                behind / total,
                BridgeTranscribeStage::Failed,
                0.0,
                0.0,
            ));
            continue;
        };
        let mut last_report = -1.0f64;
        let mut report = |seconds_in: f64| {
            let wall = started.elapsed().as_secs_f64();
            if wall - last_report < REPORT_EVERY {
                return;
            }
            last_report = wall;
            let heard = behind + seconds_in;
            let speed = if wall > 0.0 { heard / wall } else { 0.0 };
            let remaining = if speed > 0.0 {
                (total - heard).max(0.0) / speed
            } else {
                0.0
            };
            say(event(
                item,
                heard / total,
                BridgeTranscribeStage::Listening,
                speed,
                remaining,
            ));
        };
        let heard = match hear(&plan, &mut speech, &heard_options, cancel, &mut report) {
            Ok(heard) => heard,
            Err(Stop::Cancelled) => {
                outcome.cancelled = true;
                break;
            }
            Err(Stop::Missing) => {
                say(event(item, 1.0, BridgeTranscribeStage::Failed, 0.0, 0.0));
                return Err(BridgeError::AddonMissing);
            }
            Err(Stop::Failed) => {
                outcome.failed += 1;
                behind += plan.seconds;
                say(event(
                    item,
                    behind / total,
                    BridgeTranscribeStage::Failed,
                    0.0,
                    0.0,
                ));
                continue;
            }
        };
        behind += plan.seconds;
        if !heard.voices.is_empty() {
            say(event(
                item,
                behind / total,
                BridgeTranscribeStage::Speakers,
                0.0,
                0.0,
            ));
        }
        let transcript =
            transcript_of_heard(&plan, heard, &model, &heard_options, options, &mut voices);
        let landed = super::commit(
            project,
            Op::EditTranscript {
                item: plan.item,
                edit: Box::new(TranscriptEdit::Set(Some(Arc::new(transcript)))),
            },
        );
        if landed.is_ok() {
            outcome.done += 1;
            say(event(
                item,
                behind / total,
                BridgeTranscribeStage::Done,
                0.0,
                0.0,
            ));
        } else {
            outcome.failed += 1;
            say(event(
                item,
                behind / total,
                BridgeTranscribeStage::Failed,
                0.0,
                0.0,
            ));
        }
    }
    if outcome.cancelled {
        say(event(
            first,
            behind / total,
            BridgeTranscribeStage::Cancelled,
            0.0,
            0.0,
        ));
    }
    Ok(outcome)
}

/// What to hear for each item, or `None` for one that cannot be heard: not
/// footage, or a file that is not there.
fn plan(project: Uuid, items: &[Uuid], options: &BridgeTranscribeOptions) -> Vec<Option<Plan>> {
    // What can be read off the document, under the project's lock: each
    // file's path, how it is read, its stored fingerprint and its stream.
    let found: Vec<Option<Found>> = {
        let Ok(projects) = PROJECTS.read() else {
            return items.iter().map(|_| None).collect();
        };
        let Some(state) = projects.get(&project) else {
            return items.iter().map(|_| None).collect();
        };
        let Ok(state) = state.read() else {
            return items.iter().map(|_| None).collect();
        };
        let doc = state.store.snapshot();
        items
            .iter()
            .map(|id| {
                let Some(ProjectItem::Footage(footage)) = doc.item(*id) else {
                    return None;
                };
                Some(Found {
                    item: *id,
                    path: FootageReference::resolve_path(&state, footage)?,
                    source: source_of(&state, footage),
                    fingerprint: footage.media.fingerprint.clone(),
                    stream: options.stream.unwrap_or_else(|| heard_stream(&doc, *id)),
                })
            })
            .collect()
    };
    // The file reads after the lock has gone: a fingerprint reads the
    // file's ends, and a probe may read much more.
    found
        .into_iter()
        .map(|found| {
            let found = found?;
            let fingerprint = found
                .fingerprint
                .or_else(|| lumit_project::fingerprint_path(&found.path).ok());
            Some(Plan {
                item: found.item,
                seconds: seconds_of(found.source).max(0.0),
                cache: fingerprint.and_then(|f| cache_path(&f, found.stream, options)),
                path: found.path,
                stream: found.stream,
            })
        })
        .collect()
}

/// One item as the document has it, read under the lock.
struct Found {
    item: Uuid,
    path: PathBuf,
    source: Option<Source>,
    fingerprint: Option<lumit_core::model::Fingerprint>,
    stream: u32,
}

#[cfg(feature = "media")]
type Source = lumit_media::sequence::MediaSource;
#[cfg(not(feature = "media"))]
type Source = ();

#[cfg(feature = "media")]
fn source_of(
    state: &crate::api::state::LumitBridgeState,
    footage: &lumit_core::model::FootageItem,
) -> Option<Source> {
    FootageReference::resolve_source(state, footage)
}

#[cfg(not(feature = "media"))]
fn source_of(
    _state: &crate::api::state::LumitBridgeState,
    _footage: &lumit_core::model::FootageItem,
) -> Option<Source> {
    None
}

/// How long the file's sound runs, from its probe.
#[cfg(feature = "media")]
fn seconds_of(source: Option<Source>) -> f64 {
    source
        .and_then(crate::probe::ensure_probed)
        .map_or(0.0, |p| p.duration_seconds)
}

#[cfg(not(feature = "media"))]
fn seconds_of(_source: Option<Source>) -> f64 {
    0.0
}

/// The audio stream the project's layers play of `item`: the first one a
/// layer or a clip of it sounds, or the file's first. A screen recording's
/// microphone is heard, not its game, when that is what the edit plays.
fn heard_stream(doc: &Document, item: Uuid) -> u32 {
    for entry in &doc.items {
        let ProjectItem::Composition(comp) = entry else {
            continue;
        };
        for layer in &comp.layers {
            match &layer.kind {
                lumit_core::model::LayerKind::Footage { item: own } if *own == item => {
                    return layer.audio_streams.places().first().copied().unwrap_or(0);
                }
                lumit_core::model::LayerKind::Sequence { clips } => {
                    let source = lumit_core::sequence::ClipSource::Footage(item);
                    if let Some(clip) = clips.iter().find(|c| c.source == source) {
                        return clip.audio_streams.places().first().copied().unwrap_or(0);
                    }
                }
                _ => {}
            }
        }
    }
    0
}

/// Where what the model heard of one file is kept: named by the file's
/// content, the stream, the pack, the processor it runs on and the
/// options, so any change asks the model again.
fn cache_path(
    fingerprint: &lumit_core::model::Fingerprint,
    stream: u32,
    options: &BridgeTranscribeOptions,
) -> Option<PathBuf> {
    let pack = lumit_ml::speech::pick(options.translate)?;
    let pack = lumit_ml::store::identity(&pack.manifest);
    let pack: String = pack.iter().take(8).map(|b| format!("{b:02x}")).collect();
    let hash: String = fingerprint.head_tail_hash.chars().take(32).collect();
    let language = Some(options.language.trim())
        .filter(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or("auto");
    // Words to expect change what is heard, so they are part of the name.
    let words = options
        .vocabulary
        .trim()
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
        });
    // How many speakers were asked for decides how the voices are told
    // apart, and the processor can change what is heard in the last place.
    let name = format!(
        "{hash}-{size}-{stream}-{pack}-{on}-{language}-{task}-{voices}-{words:016x}.json",
        size = fingerprint.size,
        on = lumit_ml::runtime::provider().to_ascii_lowercase(),
        task = if options.translate { "t" } else { "x" },
        voices = options.speakers,
    );
    Some(lumit_project::cache_dir()?.join("speech").join(name))
}

pub(super) enum Stop {
    // Only heard where the build can decode sound.
    #[cfg_attr(not(feature = "media"), allow(dead_code))]
    Cancelled,
    Failed,
    /// The runtime or the pack would not open.
    Missing,
}

/// One file's words: from the cache when it has them, else heard.
fn hear(
    plan: &Plan,
    speech: &mut Option<lumit_ml::speech::Speech>,
    options: &lumit_ml::speech::Options,
    cancel: &AtomicBool,
    report: &mut dyn FnMut(f64),
) -> Result<lumit_ml::speech::Heard, Stop> {
    if let Some(cached) = plan
        .cache
        .as_ref()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    {
        return Ok(cached);
    }
    if speech.is_none() {
        *speech =
            Some(lumit_ml::speech::Speech::open(options.translate).map_err(|_| Stop::Missing)?);
    }
    let Some(speech) = speech.as_mut() else {
        return Err(Stop::Missing);
    };
    let heard = decode_and_hear(plan, speech, options, cancel, report)?;
    if let Some(path) = plan.cache.as_ref() {
        // A cache that will not write costs the next run some time and
        // nothing else.
        if let (Some(dir), Ok(bytes)) = (path.parent(), serde_json::to_vec(&heard)) {
            let _ = std::fs::create_dir_all(dir);
            let staged = path.with_extension("part");
            if std::fs::write(&staged, bytes).is_ok() {
                let _ = std::fs::rename(&staged, path);
            }
        }
    }
    Ok(heard)
}

#[cfg(feature = "media")]
fn decode_and_hear(
    plan: &Plan,
    speech: &mut lumit_ml::speech::Speech,
    options: &lumit_ml::speech::Options,
    cancel: &AtomicBool,
    report: &mut dyn FnMut(f64),
) -> Result<lumit_ml::speech::Heard, Stop> {
    let mut reader = lumit_media::audio::AudioReader::open_stream(&plan.path, plan.stream, RATE)
        .map_err(|_| Stop::Failed)?;
    let mut run = speech.start(options).map_err(|_| Stop::Failed)?;
    let mut at: u64 = 0;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(Stop::Cancelled);
        }
        let (block, _) = reader.read(at, BLOCK, true).map_err(|_| Stop::Failed)?;
        if block.is_empty() {
            break;
        }
        at += (block.len() / 2) as u64;
        run.feed(&lumit_audio::mix::downmix_to_mono(&block))
            .map_err(|_| Stop::Failed)?;
        report(run.heard());
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(Stop::Cancelled);
    }
    run.finish().map_err(|_| Stop::Failed)
}

#[cfg(not(feature = "media"))]
fn decode_and_hear(
    _plan: &Plan,
    _speech: &mut lumit_ml::speech::Speech,
    _options: &lumit_ml::speech::Options,
    _cancel: &AtomicBool,
    _report: &mut dyn FnMut(f64),
) -> Result<lumit_ml::speech::Heard, Stop> {
    Err(Stop::Failed)
}

/// What the model heard, as the document keeps it: times on the model's
/// own sample grid, in order and never overlapping, each word's voice
/// turned into the job's speaker for it.
fn transcript_of_heard(
    plan: &Plan,
    heard: lumit_ml::speech::Heard,
    model: &str,
    heard_options: &lumit_ml::speech::Options,
    options: &BridgeTranscribeOptions,
    voices: &mut super::voices::Voices,
) -> Transcript {
    let count = match heard_options.speakers {
        lumit_ml::speech::Speakers::Count(n) => Some(n),
        _ => None,
    };
    let by_voice = if options.speakers == 1 {
        Vec::new()
    } else {
        voices.speakers_for(heard.voices.clone(), count, &options.speaker_name)
    };
    // One voice asked for: everybody is one speaker.
    let only = (options.speakers == 1).then(|| voices.only(&options.speaker_name));
    let grid = |t: f64| {
        SourceTime(
            Rational::from_f64_on_grid(t.max(0.0), i64::from(RATE)).unwrap_or(Rational::ZERO),
        )
    };
    let mut words: Vec<Word> = Vec::with_capacity(heard.words.len());
    for w in heard.words {
        let text = w.text.trim().to_owned();
        if text.is_empty() {
            continue;
        }
        let mut start = grid(w.start);
        let mut end = grid(w.end);
        if let Some(last) = words.last() {
            start = start.max(last.end);
        }
        end = end.max(start);
        let confidence = if w.confidence == 0.0 {
            0.0
        } else {
            w.confidence.clamp(0.01, 1.0)
        };
        let mut word = Word::new(text, start, end, confidence);
        word.speaker = only.or_else(|| w.voice.and_then(|v| by_voice.get(v).copied()));
        word.paragraph = w.paragraph
            || words
                .last()
                .is_some_and(|last| last.speaker.is_some() && last.speaker != word.speaker);
        words.push(word);
    }
    let used: Vec<Uuid> = words.iter().filter_map(|w| w.speaker).collect();
    let speakers = voices.named(&used);
    Transcript {
        language: heard.language,
        stream: plan.stream,
        words,
        speakers,
        model: model.to_owned(),
        extra: serde_json::Map::new(),
    }
}

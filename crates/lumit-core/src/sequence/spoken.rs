//! The words a composition says, where it says them, and how the Transcript
//! panel lays them out for editing a cut by its text.
//!
//! # In plain terms
//!
//! A transcript belongs to a footage item and runs on the item's own clock.
//! A composition plays pieces of many items, trimmed, moved, sped up and
//! nested, so the words it says are those transcripts read through every clip
//! that is heard. [`heard`] does that reading, and [`tokens`] lays the words
//! out on the composition's frames with the pauses and breaks between them,
//! so deleting a run of text is deleting a stretch of the cut.
//!
//! # Thread role and contract
//!
//! Any thread. Pure: a document in, data out, nothing kept.

use std::collections::BTreeMap;

use crate::model::{Composition, Document, Layer, LayerKind};
use crate::sequence::{Clip, ClipSource};
use crate::time::{CompTime, FrameRate, Rational, SourceTime};
use uuid::Uuid;

pub mod fillers;
mod layout;
pub mod script;
#[cfg(test)]
mod tests;

use fillers::Filler;

/// How deep a composition inside a composition is listened through. A clip
/// can play a comp that plays a comp, and a cycle must stop somewhere.
const DEPTH: u8 = 8;

/// One word as the composition says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Heard {
    /// The footage item whose transcript holds the word.
    pub item: Uuid,
    /// The word's place in that transcript.
    pub index: usize,
    /// The top-level layer it is heard through.
    pub row: Uuid,
    /// The clip on that layer, or `None` for a Footage layer.
    pub clip: Option<Uuid>,
    /// Where it is said, in comp time, held inside its clip.
    pub start: Rational,
    pub end: Rational,
}

/// How the panel looks at the words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Look {
    /// The shortest silence shown as a pause or a break.
    pub pause: Rational,
    /// Show the words a delete or a lift took out, at the edit point.
    pub show_removed: bool,
}

impl Default for Look {
    fn default() -> Self {
        Self {
            pause: Rational::new(1, 2).unwrap_or(Rational::ONE),
            show_removed: false,
        }
    }
}

/// What a token is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// A word.
    Word,
    /// A silence between two words of one clip.
    Pause,
    /// A stretch with no words: between clips, before the first word or
    /// after the last.
    Break,
    /// A word a delete or a lift took out, shown at the edit point. It takes
    /// no time.
    Removed,
}

/// A word in a transcript, by item and place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WordAt {
    pub item: Uuid,
    pub index: usize,
}

/// One piece of the transcript as the panel shows it and an edit takes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    /// The stretch an edit of this token takes, on the comp's frame edges.
    /// Tokens tile with no holes and no overlaps.
    pub start: Rational,
    pub end: Rational,
    /// Where the word is actually heard. For a pause, its whole silence, from
    /// the latest end of the words before to the next word's start. `None`
    /// for a break or a removed word.
    pub heard: Option<(Rational, Rational)>,
    pub word: Option<WordAt>,
    /// The top-level layer, and the clip on it, the token is heard through.
    pub row: Option<Uuid>,
    pub clip: Option<Uuid>,
    pub filler: Option<Filler>,
    /// Removing this token would cut into a neighbouring word.
    pub tight: bool,
    /// An edit point falls between this token and the one before.
    pub edit_before: bool,
}

/// One word read off some composition's rows, before it is heard further up.
#[derive(Debug, Clone, Copy)]
struct Said {
    item: Uuid,
    index: usize,
    start: Rational,
    end: Rational,
}

impl Said {
    fn middle(&self) -> Rational {
        middle(self.start, self.end)
    }
}

/// Halfway between two times. The earlier one if the sum will not fit.
#[must_use]
pub fn middle(a: Rational, b: Rational) -> Rational {
    let half = Rational::new(1, 2).unwrap_or(Rational::ZERO);
    a.checked_add(b)
        .and_then(|sum| sum.checked_mul(half))
        .unwrap_or(a)
}

/// The words `comp` says, in comp time, sorted by when they start and then by
/// the stack, with mic bleed taken out: the same word heard on two rows at
/// once is kept on the row that heard it best.
#[must_use]
pub fn heard(doc: &Document, comp: &Composition) -> Vec<Heard> {
    listen(doc, comp, 0, &mut BTreeMap::new())
        .into_iter()
        .map(|(h, _)| h)
        .collect()
}

/// The layers of `comp` that are heard, with their place in the stack:
/// Sequence layers, and Footage layers with no Retime, which convert when an
/// edit reaches them. A soloed layer that makes a sound leaves only the
/// soloed ones heard, as the mixer has it.
fn heard_layers(comp: &Composition) -> impl Iterator<Item = (usize, &Layer)> {
    let solo = comp.layers.iter().any(|l| l.switches.solo && sounding(l));
    comp.layers.iter().enumerate().filter(move |(_, l)| {
        let kind = match &l.kind {
            LayerKind::Sequence { .. } => true,
            LayerKind::Footage { .. } => l.retime.is_none(),
            _ => false,
        };
        kind && l.switches.audible && (!solo || l.switches.solo)
    })
}

/// Whether a layer can make a sound, for solo. The media is not read here, so
/// any footage or composition counts, and a Sequence layer counts while one
/// of its clips is heard.
fn sounding(layer: &Layer) -> bool {
    layer.audio_only
        || match &layer.kind {
            LayerKind::Footage { .. } | LayerKind::Precomp { .. } => true,
            LayerKind::Sequence { clips } => clips.iter().any(|c| c.audible),
            _ => false,
        }
}

/// Every word heard on `comp`'s rows, each beside its row's place in the
/// stack, sorted by start and then by the stack, with mic bleed taken out.
/// `nested` holds what each nested composition says, heard once.
fn listen(
    doc: &Document,
    comp: &Composition,
    depth: u8,
    nested: &mut BTreeMap<Uuid, Vec<Said>>,
) -> Vec<(Heard, usize)> {
    let mut out = Vec::new();
    for (stack, layer) in heard_layers(comp) {
        let offset = layer.start_offset.0;
        let window = (layer.in_point.0, layer.out_point.0);
        let mut push = |said: Said, clip: Option<Uuid>| {
            let (Ok(start), Ok(end)) =
                (said.start.checked_add(offset), said.end.checked_add(offset))
            else {
                return;
            };
            let at = middle(start, end);
            if at < window.0 || at >= window.1 {
                return;
            }
            let heard = Heard {
                item: said.item,
                index: said.index,
                row: layer.id,
                clip,
                start: start.max(window.0),
                end: end.min(window.1),
            };
            out.push((heard, stack));
        };
        match &layer.kind {
            LayerKind::Footage { item } => {
                // A Footage layer plays its file from its own zero, so layer
                // time is source time.
                let (Ok(from), Ok(to)) =
                    (window.0.checked_sub(offset), window.1.checked_sub(offset))
                else {
                    continue;
                };
                for said in footage_words(doc, *item, layer.audio_streams.places(), from, to) {
                    push(said, None);
                }
            }
            LayerKind::Sequence { clips } => {
                for clip in clips.iter().filter(|c| c.audible) {
                    for said in clip_words(doc, clip, depth, nested) {
                        push(said, Some(clip.id));
                    }
                }
            }
            _ => {}
        }
    }
    out.sort_by_key(|(h, stack)| (h.start, *stack));
    let bleed = bleed(doc, &out);
    out.into_iter()
        .zip(bleed)
        .filter_map(|(word, gone)| (!gone).then_some(word))
        .collect()
}

/// The footage items `comp`'s heard clips and layers play, nested ones too,
/// each once, in the order first met.
#[must_use]
pub fn sources(doc: &Document, comp: &Composition) -> Vec<Uuid> {
    let mut out = Vec::new();
    gather(doc, comp, 0, &mut out);
    out
}

fn gather(doc: &Document, comp: &Composition, depth: u8, out: &mut Vec<Uuid>) {
    for (_, layer) in heard_layers(comp) {
        let sources: Vec<ClipSource> = match &layer.kind {
            LayerKind::Footage { item } => vec![ClipSource::Footage(*item)],
            LayerKind::Sequence { clips } => clips
                .iter()
                .filter(|c| c.audible)
                .map(|c| c.source)
                .collect(),
            _ => Vec::new(),
        };
        for source in sources {
            match source {
                ClipSource::Footage(item) if !out.contains(&item) => out.push(item),
                ClipSource::Comp(id) if depth < DEPTH => {
                    if let Some(nested) = doc.comp(id) {
                        gather(doc, nested, depth + 1, out);
                    }
                }
                _ => {}
            }
        }
    }
}

/// The words of `item`'s transcript, in its source time, any part of which
/// falls between `from` and `to`, when the transcript heard one of `streams`.
fn footage_words(
    doc: &Document,
    item: Uuid,
    streams: &[u32],
    from: Rational,
    to: Rational,
) -> Vec<Said> {
    let Some(transcript) = doc.transcripts.get(&item) else {
        return Vec::new();
    };
    if !streams.contains(&transcript.stream) {
        return Vec::new();
    }
    let range = transcript.words_between(SourceTime(from), SourceTime(to));
    let words = transcript.words.get(range.clone()).unwrap_or_default();
    words
        .iter()
        .zip(range)
        .map(|(w, index)| Said {
            item,
            index,
            start: w.start.0,
            end: w.end.0,
        })
        .collect()
}

/// The words one clip says, in its layer's time, held inside the clip.
/// `nested` is as [`listen`] has it.
fn clip_words(
    doc: &Document,
    clip: &Clip,
    depth: u8,
    nested: &mut BTreeMap<Uuid, Vec<Said>>,
) -> Vec<Said> {
    let Some(map) = Map::of(clip) else {
        return Vec::new();
    };
    let (lo, hi) = map.reach();
    let words = match clip.source {
        ClipSource::Footage(item) => footage_words(doc, item, clip.audio_streams.places(), lo, hi),
        ClipSource::Comp(id) => {
            let Some(comp) = doc.comp(id).filter(|_| depth < DEPTH) else {
                return Vec::new();
            };
            // multicam: a multicam clip hears its angle's row alone, so the
            // filter on the nested words goes here.
            if !nested.contains_key(&id) {
                let words: Vec<Said> = listen(doc, comp, depth + 1, nested)
                    .into_iter()
                    .map(|(h, _)| Said {
                        item: h.item,
                        index: h.index,
                        start: h.start,
                        end: h.end,
                    })
                    .collect();
                nested.insert(id, words);
            }
            // Sorted by start already, so only those starting before the
            // reach's end are looked at.
            let all = nested.get(&id).map(Vec::as_slice).unwrap_or_default();
            let to = all.partition_point(|w| w.start < hi);
            all.get(..to)
                .unwrap_or_default()
                .iter()
                .filter(|w| w.end >= lo)
                .copied()
                .collect()
        }
        ClipSource::Solid(_) => return Vec::new(),
    };
    let (start, end) = (clip.place_start, clip.place_end());
    let mut out = Vec::with_capacity(words.len());
    for word in words {
        for (a, b) in map.place(&word) {
            let (a, b) = (a.max(start), b.min(end));
            if a <= b {
                out.push(Said {
                    start: a,
                    end: b,
                    ..word
                });
            }
        }
    }
    out
}

/// How a clip turns source time into its layer's time, run backwards.
enum Map {
    /// Not retimed: the source runs beside the clip from `source_in`.
    Plain {
        place: Rational,
        from: Rational,
        length: Rational,
    },
    /// One speed, forwards: `s(t) = from + speed · (t - place_start)`.
    Speed {
        place: Rational,
        from: f64,
        speed: f64,
        length: f64,
    },
    /// A ramp, sampled, as the stretches where the source runs forwards. A
    /// frozen or reversed stretch says no words.
    Ramp {
        place: Rational,
        runs: Vec<Vec<(f64, f64)>>,
    },
}

/// How finely a ramp is sampled: per second of clip up to a total, then at
/// least so many per second of a longer clip, up to a ceiling.
const RAMP_SAMPLES_PER_SECOND: f64 = 100.0;
const RAMP_SAMPLES: f64 = 4096.0;
const RAMP_LEAST_PER_SECOND: f64 = 4.0;
const RAMP_MOST_SAMPLES: f64 = 65_536.0;

impl Map {
    fn of(clip: &Clip) -> Option<Map> {
        if clip.retime.is_none() {
            return Some(Map::Plain {
                place: clip.place_start,
                from: clip.source_in,
                length: clip.place_duration,
            });
        }
        let length = clip.place_duration.to_f64();
        if length <= 0.0 {
            return None;
        }
        let map = clip.effective_retime();
        if let Some(speed) = clip.constant_speed() {
            if speed <= 0.0 {
                return None;
            }
            return Some(Map::Speed {
                place: clip.place_start,
                from: map.value_at(0.0),
                speed,
                length,
            });
        }
        let n = (length * RAMP_SAMPLES_PER_SECOND)
            .min(RAMP_SAMPLES)
            .max(length * RAMP_LEAST_PER_SECOND)
            .ceil()
            .clamp(2.0, RAMP_MOST_SAMPLES) as usize;
        let samples: Vec<(f64, f64)> = (0..=n)
            .map(|i| {
                let t = length * i as f64 / n as f64;
                (t, map.value_at(t))
            })
            .collect();
        let mut runs: Vec<Vec<(f64, f64)>> = Vec::new();
        let mut open = false;
        for pair in samples.windows(2) {
            let forwards = pair[1].1 > pair[0].1 + 1e-9;
            match (forwards, open, runs.last_mut()) {
                (true, true, Some(run)) => run.push(pair[1]),
                (true, _, _) => runs.push(vec![pair[0], pair[1]]),
                (false, _, _) => {}
            }
            open = forwards;
        }
        Some(Map::Ramp {
            place: clip.place_start,
            runs,
        })
    }

    /// The stretch of source the clip plays, lowest to highest.
    fn reach(&self) -> (Rational, Rational) {
        let grid =
            |v: f64| Rational::from_f64_on_grid(v, Rational::FLICK_DEN).unwrap_or(Rational::ZERO);
        match self {
            Map::Plain { from, length, .. } => (*from, from.checked_add(*length).unwrap_or(*from)),
            Map::Speed {
                from,
                speed,
                length,
                ..
            } => (grid(*from), grid(from + speed * length)),
            Map::Ramp { runs, .. } => {
                let ends = runs
                    .iter()
                    .flat_map(|run| run.first().into_iter().chain(run.last()));
                let lo = ends.clone().map(|p| p.1).fold(f64::INFINITY, f64::min);
                let hi = ends.map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
                if lo > hi {
                    (Rational::ZERO, Rational::ZERO)
                } else {
                    (grid(lo), grid(hi))
                }
            }
        }
    }

    /// Where a word of the source is said on the layer: once, or once for
    /// every forward stretch of a ramp that plays its middle. Nothing when
    /// its middle is not played.
    fn place(&self, word: &Said) -> Vec<(Rational, Rational)> {
        let grid = |v: f64| Rational::from_f64_on_grid(v, Rational::FLICK_DEN).ok();
        let at = word.middle();
        match self {
            Map::Plain {
                place,
                from,
                length,
            } => {
                let end = from.checked_add(*length).unwrap_or(*from);
                if at < *from || at >= end {
                    return Vec::new();
                }
                let shift =
                    |s: Rational| s.checked_sub(*from).and_then(|d| place.checked_add(d)).ok();
                shift(word.start).zip(shift(word.end)).into_iter().collect()
            }
            Map::Speed {
                place,
                from,
                speed,
                length,
            } => {
                let t = |s: Rational| (s.to_f64() - from) / speed;
                let mid = t(at);
                if mid < 0.0 || mid >= *length {
                    return Vec::new();
                }
                let shift =
                    |v: f64| grid(v.clamp(0.0, *length)).and_then(|d| place.checked_add(d).ok());
                shift(t(word.start))
                    .zip(shift(t(word.end)))
                    .into_iter()
                    .collect()
            }
            Map::Ramp { place, runs } => {
                let mut out = Vec::new();
                for run in runs {
                    let (Some(first), Some(last)) = (run.first(), run.last()) else {
                        continue;
                    };
                    let mid = at.to_f64();
                    if mid < first.1 || mid >= last.1 {
                        continue;
                    }
                    let t = |s: f64| invert(run, s);
                    let shift = |v: f64| grid(v).and_then(|d| place.checked_add(d).ok());
                    let span = shift(t(word.start.to_f64())).zip(shift(t(word.end.to_f64())));
                    out.extend(span);
                }
                out
            }
        }
    }
}

/// The clip time a forward run of samples reaches source `s` at, held to the
/// run's ends.
fn invert(run: &[(f64, f64)], s: f64) -> f64 {
    let after = run.partition_point(|p| p.1 <= s);
    match (
        after.checked_sub(1).and_then(|i| run.get(i)),
        run.get(after),
    ) {
        (Some(a), Some(b)) => a.0 + (b.0 - a.0) * (s - a.1) / (b.1 - a.1),
        (Some(a), None) => a.0,
        (None, Some(b)) => b.0,
        (None, None) => 0.0,
    }
}

/// Which of `words` (sorted by start) are mic bleed: the same word heard on
/// another row at the same moment, more faintly. Of two that overlap by more
/// than half the shorter and say the same thing, the one the model was less
/// sure of goes.
fn bleed(doc: &Document, words: &[(Heard, usize)]) -> Vec<bool> {
    let read = |h: &Heard| {
        let word = doc.transcripts.get(&h.item)?.words.get(h.index)?;
        Some((fillers::fold(&word.text), word.confidence))
    };
    let said: Vec<Option<(String, f32)>> = words.iter().map(|(h, _)| read(h)).collect();
    let mut gone = vec![false; words.len()];
    for (i, (a, _)) in words.iter().enumerate() {
        for (j, (b, _)) in words.iter().enumerate().skip(i + 1) {
            if b.start >= a.end {
                break;
            }
            if a.row == b.row || gone[i] || gone[j] {
                continue;
            }
            let (Some((ta, ca)), Some((tb, cb))) = (&said[i], &said[j]) else {
                continue;
            };
            let shared = a.end.min(b.end).checked_sub(a.start.max(b.start));
            let shorter = a
                .end
                .checked_sub(a.start)
                .ok()
                .zip(b.end.checked_sub(b.start).ok())
                .map(|(x, y)| x.min(y));
            let (Ok(shared), Some(shorter)) = (shared, shorter) else {
                continue;
            };
            let overlaps = shared
                .checked_add(shared)
                .is_ok_and(|twice| twice > shorter);
            if overlaps && ta == tb {
                if cb > ca {
                    gone[i] = true;
                } else {
                    gone[j] = true;
                }
            }
        }
    }
    gone
}

/// The transcript of `comp` as the panel shows it and as edits take it: its
/// heard words, with the pauses and breaks between them, tiled on its frames
/// from the first heard clip's start to the last clip's end.
#[must_use]
pub fn tokens(doc: &Document, comp: &Composition, look: &Look) -> Vec<Token> {
    let heard = heard(doc, comp);
    // Edit points and the tile's start come from the clips that are heard:
    // b-roll on a heard picture row is silent, and its cuts are not the
    // speech's. The tile ends at the last clip of any row.
    let sounds = |_: ClipSource, _: &[u32], audible: bool| audible;
    let speaks = |source: ClipSource, streams: &[u32], audible: bool| {
        audible && transcribed(doc, source, streams)
    };
    let mut edits: Vec<Rational> = Vec::new();
    let mut covered: Vec<(Rational, Rational)> = Vec::new();
    let mut ends: Vec<Rational> = Vec::new();
    for (_, layer) in heard_layers(comp) {
        for (a, b) in spans(layer, &sounds) {
            edits.extend([a, b]);
        }
        covered.extend(spans(layer, &speaks));
    }
    for layer in &comp.layers {
        if matches!(layer.kind, LayerKind::Sequence { .. }) {
            ends.extend(spans(layer, &|_, _, _| true).into_iter().map(|(_, b)| b));
        }
    }
    edits.sort();
    edits.dedup();
    let (Some(first), Some(last)) = (edits.first(), ends.iter().chain(edits.last()).max()) else {
        return Vec::new();
    };
    let tile = (*first, *last);
    let mut tokens = layout::lay_out(
        doc,
        &heard,
        tile,
        &edits,
        &merged(covered),
        comp.frame_rate,
        look,
    );
    if look.show_removed {
        layout::add_removed(doc, comp, &mut tokens);
    }
    tokens
}

/// Whether a clip of `source` sounding `streams` says words a transcript
/// holds: a footage item transcribed on one of those streams, or a
/// composition that plays one.
fn transcribed(doc: &Document, source: ClipSource, streams: &[u32]) -> bool {
    match source {
        ClipSource::Footage(item) => doc
            .transcripts
            .get(&item)
            .is_some_and(|t| streams.contains(&t.stream)),
        ClipSource::Comp(id) => doc.comp(id).is_some_and(|nested| {
            sources(doc, nested)
                .iter()
                .any(|i| doc.transcripts.contains_key(i))
        }),
        ClipSource::Solid(_) => false,
    }
}

/// The transcript of footage item `item` on its own clock, as
/// [`tokens`] lays a composition's out: the whole file one stretch of
/// speech, so the silences between words are pauses. On `rate`'s frames, and
/// with no rows, clips or removed words.
#[must_use]
pub fn source_tokens(doc: &Document, item: Uuid, rate: FrameRate, look: &Look) -> Vec<Token> {
    let Some(transcript) = doc.transcripts.get(&item) else {
        return Vec::new();
    };
    let heard: Vec<Heard> = transcript
        .words
        .iter()
        .enumerate()
        .map(|(index, w)| Heard {
            item,
            index,
            row: item,
            clip: None,
            start: w.start.0,
            end: w.end.0,
        })
        .collect();
    let Some(last) = heard.last().map(|h| h.end) else {
        return Vec::new();
    };
    // The whole file is speech, so every silence between words is a pause.
    let whole = [(Rational::ZERO, last)];
    let mut tokens = layout::lay_out(doc, &heard, (Rational::ZERO, last), &[], &whole, rate, look);
    for token in &mut tokens {
        token.row = None;
    }
    tokens
}

/// Where a layer's clips that `keep` picks sit in comp time, held to its in
/// and out points. A Footage layer is one span from its in point, or from its
/// own zero when that is later. `keep` is asked of each clip's source, the
/// streams it sounds and whether it is heard.
fn spans(
    layer: &Layer,
    keep: &dyn Fn(ClipSource, &[u32], bool) -> bool,
) -> Vec<(Rational, Rational)> {
    let offset = layer.start_offset.0;
    let (lo, hi) = (layer.in_point.0, layer.out_point.0);
    let held = |a: Rational, b: Rational| {
        let (a, b) = (a.max(lo), b.min(hi));
        (a < b).then_some((a, b))
    };
    match &layer.kind {
        LayerKind::Sequence { clips } => clips
            .iter()
            .filter(|c| keep(c.source, c.audio_streams.places(), c.audible))
            .filter_map(|c| {
                let a = c.place_start.checked_add(offset).ok()?;
                let b = c.place_end().checked_add(offset).ok()?;
                held(a, b)
            })
            .collect(),
        LayerKind::Footage { item }
            if keep(
                ClipSource::Footage(*item),
                layer.audio_streams.places(),
                true,
            ) =>
        {
            held(offset.max(lo), hi).into_iter().collect()
        }
        _ => Vec::new(),
    }
}

/// The frame edge nearest `t` on `rate`.
fn edge(rate: FrameRate, t: Rational) -> Rational {
    rate.time_of_frame(rate.nearest_frame_at(CompTime(t)))
        .map_or(t, |at| at.0)
}

/// The frame edge at or before `t` on `rate`, or with `up` the one at or
/// after it.
fn edge_toward(rate: FrameRate, t: Rational, up: bool) -> Option<Rational> {
    let frame = rate.frame_at(CompTime(t));
    let floor = rate.time_of_frame(frame).ok()?.0;
    if up && floor < t {
        Some(rate.time_of_frame(frame.checked_add(1)?).ok()?.0)
    } else {
        Some(floor)
    }
}

/// The stretches to take out so every pause whose whole silence, word end to
/// word start, is longer than `longer_than` keeps `keep` of it, each centred in
/// its silence and inside it on `rate`'s frame edges, in order. A break is
/// never shortened this way: it is not a silence of one clip.
#[must_use]
pub fn shorten_pauses(
    tokens: &[Token],
    longer_than: Rational,
    keep: Rational,
    rate: FrameRate,
) -> Vec<(Rational, Rational)> {
    let mut out = Vec::new();
    for token in tokens {
        if token.kind != TokenKind::Pause {
            continue;
        }
        let Some((from, to)) = token.heard else {
            continue;
        };
        let Ok(silence) = to.checked_sub(from) else {
            continue;
        };
        if silence <= longer_than || silence <= keep {
            continue;
        }
        let Ok(half) = silence
            .checked_sub(keep)
            .and_then(|cut| cut.checked_div(Rational::new(2, 1)?))
        else {
            continue;
        };
        let centre = middle(from, to);
        let (Ok(a), Ok(b)) = (centre.checked_sub(half), centre.checked_add(half)) else {
            continue;
        };
        // In on the frame at or after, out on the one at or before, so the
        // cut never reaches a word.
        let (Some(a), Some(b)) = (edge_toward(rate, a, true), edge_toward(rate, b, false)) else {
            continue;
        };
        if a < b {
            out.push((a, b));
        }
    }
    out
}

/// `spans` in order, with any that touch or overlap made one.
#[must_use]
pub fn merged(mut spans: Vec<(Rational, Rational)>) -> Vec<(Rational, Rational)> {
    spans.retain(|(a, b)| a < b);
    spans.sort();
    let mut out: Vec<(Rational, Rational)> = Vec::with_capacity(spans.len());
    for (a, b) in spans {
        match out.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

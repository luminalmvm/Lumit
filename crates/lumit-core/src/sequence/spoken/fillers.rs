//! Filler words: the hesitations, the set-off phrases and the restarts a
//! speaker leaves in, which a cut usually takes out.
//!
//! Thread: any. Pure.

use super::Heard;
use crate::model::Document;
use crate::time::Rational;

/// Why a word is a filler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filler {
    /// A sound and not a word: "um", "uh", or a sound the model heard and did
    /// not write down.
    Hesitation,
    /// A word or phrase said to fill, set off from the sentence: "you know",
    /// "like", "so".
    Phrase,
    /// A word or a pair said again straight after: the earlier one.
    Repeat,
    /// A sentence the speaker stopped and said again soon after: every word
    /// of the earlier attempt.
    Retake,
}

/// `text` as words are compared: lower case, with its punctuation gone.
#[must_use]
pub fn fold(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// The language part of a tag: `en` of `en-GB`.
pub(super) fn base(language: &str) -> String {
    let base = language.split(['-', '_']).next().unwrap_or_default();
    match base.to_lowercase().as_str() {
        "nb" | "nn" => "no".to_owned(),
        other => other.to_owned(),
    }
}

/// The sounds a speaker of `language` fills with, folded.
pub(super) fn hesitations(language: &str) -> &'static [&'static str] {
    match language {
        "en" => &[
            "um", "uh", "uhm", "umm", "er", "erm", "ah", "eh", "hmm", "hm", "mm", "mhm",
        ],
        "de" => &["äh", "ähm", "öh", "hm", "hmm", "ähh"],
        "fr" => &["euh", "heu", "bah", "hum"],
        "es" => &["eh", "em", "ehm", "mmm"],
        "it" => &["ehm", "eh", "mmm", "uhm"],
        "pt" => &["hã", "ahn", "hum"],
        "nl" => &["eh", "ehm", "uh", "uhm"],
        "sv" => &["eh", "öh", "ehm", "hmm"],
        "da" => &["øh", "æh", "øhm", "hmm"],
        "no" => &["eh", "øh", "ehm", "hmm"],
        "pl" => &["yyy", "yy", "eee", "hmm"],
        "ru" => &["э", "ээ", "эм", "хм"],
        "tr" => &["ıı", "ee", "hmm"],
        "ja" => &["えー", "えっと", "あのー", "えーと", "うーん"],
        "zh" => &["嗯", "呃", "额"],
        "ko" => &["음", "어"],
        _ => &[],
    }
}

/// The words and phrases a speaker of `language` fills with when they are set
/// off, folded, a space between the words of a phrase.
pub(super) fn phrases(language: &str) -> &'static [&'static str] {
    match language {
        "en" => &[
            "you know",
            "i mean",
            "like",
            "sort of",
            "kind of",
            "i guess",
            "basically",
            "actually",
            "literally",
            "right",
            "so",
            "well",
        ],
        "de" => &[
            "also",
            "halt",
            "quasi",
            "sozusagen",
            "irgendwie",
            "naja",
            "weißt du",
            "ich meine",
        ],
        "fr" => &[
            "en fait",
            "genre",
            "du coup",
            "tu vois",
            "voilà",
            "bon",
            "alors",
            "je veux dire",
            "ben",
        ],
        "es" => &[
            "o sea", "pues", "bueno", "entonces", "sabes", "digamos", "en plan", "este",
        ],
        "it" => &[
            "cioè",
            "tipo",
            "allora",
            "praticamente",
            "insomma",
            "diciamo",
        ],
        "pt" => &["tipo", "então", "né", "sabe", "quer dizer", "assim", "é"],
        "nl" => &["dus", "nou", "eigenlijk", "weet je", "zeg maar", "gewoon"],
        "sv" => &["liksom", "typ", "alltså", "du vet"],
        "da" => &["altså", "ligesom", "du ved"],
        "no" => &["liksom", "altså", "typ", "du vet", "på en måte"],
        "pl" => &["jakby", "wiesz", "znaczy", "po prostu", "w sumie"],
        "ru" => &["типа", "как бы", "вот", "значит", "короче", "то есть", "ну"],
        "tr" => &["yani", "işte", "hani", "falan", "aslında", "şey"],
        "ja" => &["なんか", "やっぱり", "ちょっと", "あの"],
        "zh" => &["就是", "然后", "那么", "其实", "那个"],
        "ko" => &["뭐", "약간", "그러니까", "좀", "그", "저기"],
        _ => &[],
    }
}

/// What a word ending a sentence ends with.
const SENTENCE_ENDS: [char; 6] = ['.', '?', '!', '。', '？', '！'];

/// How far apart a word said twice may be and still be a restart.
fn restart() -> Rational {
    Rational::ONE
}

/// One word as fillers are found among them.
struct Read<'a> {
    folded: String,
    text: &'a str,
    confidence: f32,
    language: String,
    speaker: Option<uuid::Uuid>,
    row: uuid::Uuid,
    start: Rational,
    end: Rational,
}

/// Which of `heard`, in the order said, are fillers. `pause` is the silence
/// that sets a phrase off as a comma does.
pub(super) fn mark(doc: &Document, heard: &[Heard], pause: Rational) -> Vec<Option<Filler>> {
    let words: Vec<Option<Read<'_>>> = heard
        .iter()
        .map(|h| {
            let transcript = doc.transcripts.get(&h.item)?;
            let word = transcript.words.get(h.index)?;
            Some(Read {
                folded: fold(&word.text),
                text: &word.text,
                confidence: word.confidence,
                language: base(&transcript.language),
                speaker: word.speaker,
                row: h.row,
                start: h.start,
                end: h.end,
            })
        })
        .collect();
    let mut out: Vec<Option<Filler>> = vec![None; words.len()];
    let at = |i: usize| words.get(i).and_then(Option::as_ref);
    // Said again by the same speaker on the same row, and not across the end
    // of a sentence.
    let same = |i: usize, j: usize| match (at(i), at(j)) {
        (Some(a), Some(b)) => {
            !a.folded.is_empty()
                && a.folded == b.folded
                && a.speaker == b.speaker
                && a.row == b.row
                && !a.text.ends_with(SENTENCE_ENDS)
        }
        _ => false,
    };
    let soon = |i: usize, j: usize| match (at(i), at(j)) {
        (Some(a), Some(b)) => b.start.checked_sub(a.end).is_ok_and(|gap| gap <= restart()),
        _ => false,
    };
    let comma = |i: usize| at(i).is_some_and(|w| w.text.ends_with([',', '，', '、']));
    // Each language's phrases split into their words, once.
    let mut split: Vec<(&str, Vec<Vec<&str>>)> = Vec::new();
    for word in words.iter().flatten() {
        if !split.iter().any(|(language, _)| *language == word.language) {
            let parts = phrases(&word.language)
                .iter()
                .map(|phrase| phrase.split(' ').collect())
                .collect();
            split.push((&word.language, parts));
        }
    }
    for i in 0..words.len() {
        let Some(word) = at(i) else {
            continue;
        };
        if word.confidence == 0.0 || hesitations(&word.language).contains(&word.folded.as_str()) {
            if let Some(slot) = out.get_mut(i) {
                *slot = Some(Filler::Hesitation);
            }
            continue;
        }
        let restarted = if same(i, i + 1) && soon(i, i + 1) {
            1
        } else if same(i, i + 2) && same(i + 1, i + 3) && soon(i + 1, i + 2) {
            2
        } else {
            0
        };
        for slot in out.iter_mut().skip(i).take(restarted) {
            *slot = Some(slot.unwrap_or(Filler::Repeat));
        }
    }
    // The latest end of any word up to each, as `lay_out` measures silence:
    // a word on another row can still be playing after the one before ends.
    let mut latest: Option<Rational> = None;
    let ends: Vec<Option<Rational>> = words
        .iter()
        .map(|w| {
            if let Some(w) = w {
                latest = Some(latest.map_or(w.end, |l| l.max(w.end)));
            }
            latest
        })
        .collect();
    // Whether a pause follows word `i`, or no word does.
    let paused_after = |i: usize| match (ends.get(i).copied().flatten(), at(i + 1)) {
        (Some(end), Some(next)) => next.start.checked_sub(end).is_ok_and(|gap| gap >= pause),
        _ => true,
    };
    for i in 0..words.len() {
        let Some(word) = at(i) else {
            continue;
        };
        let paused = i.checked_sub(1).is_none_or(paused_after);
        let language = split
            .iter()
            .find(|(language, _)| *language == word.language);
        for parts in language
            .map(|(_, parts)| parts.as_slice())
            .unwrap_or_default()
        {
            let fits = parts
                .iter()
                .enumerate()
                .all(|(k, part)| at(i + k).is_some_and(|w| w.folded == *part));
            let last = i + parts.len() - 1;
            // Set off on both sides: by a comma, the end of a sentence or a
            // pause, or the start or end of what was said.
            let ended = |i: usize| at(i).is_some_and(|w| w.text.ends_with(SENTENCE_ENDS));
            let before = i.checked_sub(1).is_some_and(|p| comma(p) || ended(p)) || paused;
            let set_off = before && (comma(last) || ended(last) || paused_after(last));
            if fits && set_off {
                for slot in out.iter_mut().skip(i).take(parts.len()) {
                    *slot = slot.or(Some(Filler::Phrase));
                }
            }
        }
    }
    retakes(&words, &mut out);
    out
}

/// How soon a sentence has to be said again to be a retake of it.
fn retake_window() -> Rational {
    Rational::new(30, 1).unwrap_or(Rational::ONE)
}

/// The shortest sentence a retake is looked for, in words, so "Thank you."
/// said twice is left alone.
const RETAKE_WORDS: usize = 3;

/// The longest sentence compared for a retake, in words, so a transcript
/// with few full stops is not matched word against word at length.
const RETAKE_LONGEST: usize = 60;

/// The share of an earlier sentence's words the later one has to say, in
/// tenths.
const RETAKE_SHARE: usize = 7;

/// Mark as retakes the sentences a known speaker said again within
/// [`retake_window`], before anyone else spoke: every word of each earlier
/// attempt, matched word by word as a script line is. What was said between
/// the two, an apology or a "one more time", is left as it is.
fn retakes(words: &[Option<Read<'_>>], out: &mut [Option<Filler>]) {
    // Each sentence as the range of words it spans.
    let mut sentences: Vec<std::ops::Range<usize>> = Vec::new();
    let mut from = 0;
    for (i, word) in words.iter().enumerate() {
        let ends = word
            .as_ref()
            .is_some_and(|w| w.text.ends_with(SENTENCE_ENDS));
        if ends || i + 1 == words.len() {
            sentences.push(from..i + 1);
            from = i + 1;
        }
    }
    let folded = |range: &std::ops::Range<usize>| -> Vec<String> {
        words
            .get(range.clone())
            .unwrap_or_default()
            .iter()
            .flatten()
            .map(|w| w.folded.clone())
            .filter(|w| !w.is_empty())
            .collect()
    };
    for (k, earlier) in sentences.iter().enumerate() {
        if earlier.len() > RETAKE_LONGEST {
            continue;
        }
        let said = folded(earlier);
        if said.len() < RETAKE_WORDS {
            continue;
        }
        let at = |i: usize| words.get(i).and_then(Option::as_ref);
        let Some(speaker) = at(earlier.start).and_then(|w| w.speaker) else {
            continue;
        };
        let Some(end) = earlier.end.checked_sub(1).and_then(at).map(|w| w.end) else {
            continue;
        };
        for later in sentences.iter().skip(k + 1) {
            let Some(start) = at(later.start).map(|w| w.start) else {
                break;
            };
            let soon = start
                .checked_sub(end)
                .is_ok_and(|gap| gap <= retake_window());
            // Nobody else may have spoken since, and the speaker is known
            // throughout.
            let one_run = (earlier.start..later.end)
                .all(|i| at(i).is_some_and(|w| w.speaker == Some(speaker)));
            if !soon || !one_run {
                break;
            }
            if later.len() > RETAKE_LONGEST {
                continue;
            }
            let again = folded(later);
            let matched = super::script::align(&said, &again).map_or(0, |(_, _, _, same)| same);
            if matched * 10 >= said.len() * RETAKE_SHARE {
                for slot in out.iter_mut().skip(earlier.start).take(earlier.len()) {
                    *slot = Some(Filler::Retake);
                }
                break;
            }
        }
    }
}

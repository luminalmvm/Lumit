//! Finding a script's lines in what was said: for each line, the runs of
//! words in the transcripts that say it, best first, so a cut can be put
//! together from the script and the other takes of each line kept beside it.
//!
//! Thread: any. Pure.

use std::collections::{HashMap, HashSet};

use super::fillers::fold;
use crate::model::Document;
use uuid::Uuid;

/// A run of words in one item's transcript that says a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    pub item: Uuid,
    /// The first word of the run, and the word after its last.
    pub from: usize,
    pub to: usize,
    /// How well it matches: two for each word said as written, less one for
    /// each word said differently, left out or put in.
    pub score: i32,
}

/// What an alignment gives for a word said as written, and takes for one
/// said differently, left out or added.
const SAME: i32 = 2;
const OTHER: i32 = -1;
const GAP: i32 = -1;

/// The share of a line's words a run has to say, in tenths.
const ENOUGH: usize = 6;

/// How many places in the transcripts each line is looked for at, at most.
/// The places its rarest word is said come first.
const SEEDS: usize = 64;

/// Words a full stop follows without ending the sentence, folded.
const ABBREVIATIONS: &[&str] = &["dr", "mr", "mrs", "ms", "prof", "st", "vs"];

/// The lines of `script` worth looking for: split at blank lines and at the
/// end of each sentence, leaving out a line that is only a character's name
/// in capitals or a direction in brackets.
#[must_use]
pub fn lines(script: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut paragraph = String::new();
    let mut flush = |paragraph: &mut String| {
        let mut sentence = String::new();
        let mut chars = paragraph.chars().peekable();
        while let Some(c) = chars.next() {
            sentence.push(c);
            // A full stop ends a sentence before a space, a closing quote or
            // bracket, or the end, and not after a title: "3.5" and
            // "Dr. Smith" do not.
            let stop = c == '.'
                && chars.peek().is_none_or(|n| {
                    n.is_whitespace() || matches!(n, '"' | '\'' | '”' | '’' | '»' | ')' | ']')
                })
                && !sentence
                    .split_whitespace()
                    .next_back()
                    .is_some_and(|w| ABBREVIATIONS.contains(&fold(w).as_str()));
            if stop || matches!(c, '?' | '!' | '。' | '？' | '！') {
                // A closing quote or bracket stays with the sentence it closes.
                while let Some(n) =
                    chars.next_if(|n| matches!(n, '"' | '\'' | '”' | '’' | '»' | ')' | ']'))
                {
                    sentence.push(n);
                }
                keep(&mut out, &sentence);
                sentence.clear();
            }
        }
        keep(&mut out, &sentence);
        paragraph.clear();
    };
    for line in script.lines() {
        let line = line.trim();
        if line.is_empty() {
            flush(&mut paragraph);
            continue;
        }
        if is_name(line) || is_direction(line) {
            flush(&mut paragraph);
            continue;
        }
        if !paragraph.is_empty() {
            paragraph.push(' ');
        }
        paragraph.push_str(line);
    }
    flush(&mut paragraph);
    out
}

/// Add a sentence to the lines when it has a word in it.
fn keep(out: &mut Vec<String>, sentence: &str) {
    let sentence = sentence.trim();
    if sentence.split_whitespace().any(|w| !fold(w).is_empty()) {
        out.push(sentence.to_owned());
    }
}

/// A character's name as a script heads their lines: letters, every one a
/// capital, and perhaps a note in brackets after it. A script without
/// capitals has no names to find, and a line ending a sentence is dialogue.
fn is_name(line: &str) -> bool {
    let name = line.split('(').next().unwrap_or(line);
    !name.trim_end().ends_with(['.', '?', '!'])
        && name.chars().any(char::is_uppercase)
        && !name.chars().any(|c| c.is_alphabetic() && !c.is_uppercase())
}

/// A direction: a line wholly in brackets.
fn is_direction(line: &str) -> bool {
    (line.starts_with('(') && line.ends_with(')')) || (line.starts_with('[') && line.ends_with(']'))
}

/// For each of `lines`, every run of words in `items`' transcripts that says
/// enough of it, best first. Of two equally good, the later take comes
/// first: the later item in `items`, or later in the same one.
#[must_use]
pub fn find(doc: &Document, items: &[Uuid], lines: &[String]) -> Vec<Vec<Match>> {
    // Every transcript's words folded, and where each word is said.
    let said: Vec<(Uuid, Vec<String>)> = items
        .iter()
        .filter_map(|item| {
            let words = doc
                .transcripts
                .get(item)?
                .words
                .iter()
                .map(|w| fold(&w.text));
            Some((*item, words.collect()))
        })
        .collect();
    let mut index: HashMap<&str, Vec<(usize, usize)>> = HashMap::new();
    for (t, (_, words)) in said.iter().enumerate() {
        for (at, word) in words.iter().enumerate() {
            if !word.is_empty() {
                index.entry(word.as_str()).or_default().push((t, at));
            }
        }
    }
    lines
        .iter()
        .map(|line| {
            let words: Vec<String> = line
                .split_whitespace()
                .map(fold)
                .filter(|w| !w.is_empty())
                .collect();
            find_line(&said, &index, &words)
        })
        .collect()
}

/// One line's matches.
fn find_line(
    said: &[(Uuid, Vec<String>)],
    index: &HashMap<&str, Vec<(usize, usize)>>,
    line: &[String],
) -> Vec<Match> {
    if line.is_empty() {
        return Vec::new();
    }
    // The line's words the transcripts say, rarest first, and the places
    // they are said, up to the limit.
    let mut seen: Vec<&Vec<(usize, usize)>> = Vec::new();
    for word in line {
        if let Some(places) = index.get(word.as_str()) {
            if !seen.iter().any(|s| std::ptr::eq(*s, places)) {
                seen.push(places);
            }
        }
    }
    seen.sort_by_key(|places| places.len());
    let reach = line.len() * 2 + 3;
    let mut seeds: Vec<(usize, usize)> = seen
        .iter()
        .copied()
        .flatten()
        .take(SEEDS)
        .copied()
        .collect();
    // A rarest word said more often than the limit: its places where most of
    // the line is said nearby, wherever in the transcripts they are.
    if let Some(rarest) = seen.first().filter(|places| places.len() > SEEDS) {
        let wanted: HashSet<&str> = line.iter().map(String::as_str).collect();
        let mut ranked: Vec<(usize, (usize, usize))> = rarest
            .iter()
            .map(|place| {
                let words = said.get(place.0).map_or(&[][..], |(_, w)| w.as_slice());
                let near = place.1.saturating_sub(reach)..(place.1 + reach + 1).min(words.len());
                let hits = words
                    .get(near)
                    .unwrap_or_default()
                    .iter()
                    .filter(|w| wanted.contains(w.as_str()))
                    .count();
                (hits, *place)
            })
            .collect();
        ranked.sort_by_key(|(hits, _)| std::cmp::Reverse(*hits));
        seeds = ranked
            .into_iter()
            .take(SEEDS)
            .map(|(_, place)| place)
            .collect();
    }
    let mut found: Vec<(usize, Match)> = Vec::new();
    for (t, at) in seeds {
        let Some((item, words)) = said.get(t) else {
            continue;
        };
        let from = at.saturating_sub(reach);
        let to = (at + reach + 1).min(words.len());
        let Some(window) = words.get(from..to) else {
            continue;
        };
        let Some((start, end, score, same)) = align(line, window) else {
            continue;
        };
        if same * 10 < line.len() * ENOUGH {
            continue;
        }
        let run = Match {
            item: *item,
            from: from + start,
            to: from + end,
            score,
        };
        // Two seeds of one take find it twice: the better stays, and no two
        // that are kept overlap.
        let clashes = |m: &Match| m.item == run.item && m.from < run.to && run.from < m.to;
        if found
            .iter()
            .any(|(_, m)| clashes(m) && m.score >= run.score)
        {
            continue;
        }
        found.retain(|(_, m)| !clashes(m));
        found.push((t, run));
    }
    found.sort_by(|(ta, a), (tb, b)| (b.score, tb, b.from).cmp(&(a.score, ta, a.from)));
    found.into_iter().map(|(_, m)| m).collect()
}

/// The best local alignment of `line` inside `window`: where in the window
/// it starts and ends, its score, and how many words it matches as written.
/// `None` when nothing scores.
pub(super) fn align(line: &[String], window: &[String]) -> Option<(usize, usize, i32, usize)> {
    // One row of cells at a time: each holds its score, the matched words on
    // its path and where in the window that path started.
    type Cell = (i32, usize, usize);
    let width = window.len() + 1;
    let mut above: Vec<Cell> = (0..width).map(|j| (0, 0, j)).collect();
    let mut best: Option<(usize, usize, i32, usize)> = None;
    for word in line {
        let mut row: Vec<Cell> = Vec::with_capacity(width);
        row.push((0, 0, 0));
        for j in 1..width {
            let (Some(diag), Some(up), Some(left), Some(said)) = (
                above.get(j - 1),
                above.get(j),
                row.get(j - 1),
                window.get(j - 1),
            ) else {
                break;
            };
            let same = said == word;
            let candidates = [
                (
                    diag.0 + if same { SAME } else { OTHER },
                    diag.1 + usize::from(same),
                    diag.2,
                ),
                (up.0 + GAP, up.1, up.2),
                (left.0 + GAP, left.1, left.2),
            ];
            let mut cell = candidates
                .into_iter()
                .max_by_key(|c| (c.0, c.1))
                .unwrap_or((0, 0, j));
            if cell.0 <= 0 {
                cell = (0, 0, j);
            }
            if best.is_none_or(|b| (cell.0, cell.1) > (b.2, b.3)) && cell.0 > 0 {
                best = Some((cell.2, j, cell.0, cell.1));
            }
            row.push(cell);
        }
        above = row;
    }
    best
}

//! Heard words laid out as tokens on a clock's frames: the words, the pauses
//! and breaks between them, and the words a cut took out.
//!
//! Thread: any. Pure.

use super::{edge, fillers, heard_layers, middle, Heard, Look, Token, TokenKind, WordAt};
use crate::model::{Composition, Document, LayerKind};
use crate::sequence::ClipSource;
use crate::time::{FrameRate, Rational, SourceTime};

/// How much of a silence a word keeps beside it when the rest is a pause or a
/// break: a quarter, and never more than 150 ms, so a cut there does not clip
/// the word's last sound or its breath in.
fn lead(silence: Rational) -> Rational {
    let quarter = silence
        .checked_div(Rational::new(4, 1).unwrap_or(Rational::ONE))
        .unwrap_or(Rational::ZERO);
    quarter.min(Rational::new(3, 20).unwrap_or(Rational::ZERO))
}

/// A token that is not a word.
fn blank(kind: TokenKind, start: Rational, end: Rational) -> Token {
    Token {
        kind,
        start,
        end,
        heard: None,
        word: None,
        row: None,
        clip: None,
        filler: None,
        tight: false,
        edit_before: false,
    }
}

/// End the word token at `open`, if there is one, at `at`. The words after
/// it were said wholly under it, so they take no time and sit at `at`.
fn close(tokens: &mut [Token], open: Option<usize>, at: Rational) {
    let Some(o) = open else {
        return;
    };
    if let Some(token) = tokens.get_mut(o) {
        token.end = at;
    }
    for token in tokens.iter_mut().skip(o + 1) {
        token.start = at;
        token.end = at;
    }
}

/// `heard`, sorted, laid out as tokens over `tile` on `rate`'s frame edges.
/// `edits` are the edit points of the heard clips, sorted. `speech` is where
/// transcribed sound plays, merged: a silence wholly inside it is a pause,
/// the speech going on through it on whatever row, and any other is a break.
pub(super) fn lay_out(
    doc: &Document,
    heard: &[Heard],
    tile: (Rational, Rational),
    edits: &[Rational],
    speech: &[(Rational, Rational)],
    rate: FrameRate,
    look: &Look,
) -> Vec<Token> {
    let start = edge(rate, tile.0);
    let end = edge(rate, tile.1).max(start);
    let fillers = fillers::mark(doc, heard, look.pause);
    let mut tokens: Vec<Token> = Vec::with_capacity(heard.len() * 2 + 1);
    // Where the next token starts, and the word whose end waits for it.
    let mut cursor = start;
    let mut open: Option<usize> = None;
    // The latest end of any word so far: a word on another row can still be
    // playing after the word before by start has ended.
    let mut said_to = tile.0;
    for (i, h) in heard.iter().enumerate() {
        let prev = i.checked_sub(1).and_then(|p| heard.get(p));
        let from = said_to;
        said_to = said_to.max(h.end);
        let same = prev.is_some_and(|p| p.row == h.row && p.clip == h.clip);
        let silence = h.start.checked_sub(from).unwrap_or(Rational::ZERO);
        // A word said wholly under the open one takes none of its time.
        let covered = open.is_some() && h.end <= from;
        let mut placed = covered;
        if silence >= look.pause && silence > Rational::ZERO {
            let lead = lead(silence);
            let a = match prev {
                Some(_) => edge(rate, from.checked_add(lead).unwrap_or(from)),
                None => cursor,
            }
            .max(cursor);
            let b = edge(rate, h.start.checked_sub(lead).unwrap_or(h.start));
            if a < b {
                close(&mut tokens, open, a);
                let spoken = speech.iter().any(|(s, e)| *s <= from && h.start <= *e);
                tokens.push(match prev {
                    Some(p) if spoken => Token {
                        row: Some(p.row),
                        clip: p.clip,
                        heard: Some((from, h.start)),
                        ..blank(TokenKind::Pause, a, b)
                    },
                    _ => blank(TokenKind::Break, a, b),
                });
                cursor = b;
                placed = true;
            }
        }
        if !placed && prev.is_some() {
            // Between two clips the cut falls on an edit point inside the
            // silence where there is one, so a word is never split from its
            // own clip.
            let centre = middle(from, h.start);
            let distance = |e: &&Rational| {
                let d = e.checked_sub(centre).unwrap_or(Rational::ZERO);
                if d.is_negative() {
                    d.checked_neg().unwrap_or(d)
                } else {
                    d
                }
            };
            let point = if same {
                None
            } else {
                let lo = edits.partition_point(|e| *e < from);
                let hi = edits.partition_point(|e| *e <= h.start);
                edits
                    .get(lo..hi)
                    .unwrap_or_default()
                    .iter()
                    .min_by_key(distance)
                    .copied()
            };
            let b = edge(rate, point.unwrap_or(centre)).max(cursor);
            close(&mut tokens, open, b);
            cursor = b;
        }
        tokens.push(Token {
            kind: TokenKind::Word,
            start: cursor,
            end: cursor,
            heard: Some((h.start, h.end)),
            word: Some(WordAt {
                item: h.item,
                index: h.index,
            }),
            row: Some(h.row),
            clip: h.clip,
            filler: fillers.get(i).copied().flatten(),
            tight: false,
            edit_before: false,
        });
        if !covered {
            open = Some(tokens.len() - 1);
        }
    }
    let end = end.max(cursor);
    match heard.last() {
        Some(_) => {
            let silence = end.checked_sub(said_to).unwrap_or(Rational::ZERO);
            let a = edge(rate, said_to.checked_add(lead(silence)).unwrap_or(end)).max(cursor);
            if silence >= look.pause && a < end {
                close(&mut tokens, open, a);
                tokens.push(blank(TokenKind::Break, a, end));
            } else {
                close(&mut tokens, open, end);
            }
        }
        None if start < end => tokens.push(blank(TokenKind::Break, start, end)),
        None => {}
    }
    flag(&mut tokens, edits, start);
    tokens
}

/// Set each token's `edit_before` and `tight`.
fn flag(tokens: &mut [Token], edits: &[Rational], start: Rational) {
    let word_heard = |t: Option<&Token>| {
        t.filter(|t| t.kind == TokenKind::Word)
            .and_then(|t| t.heard)
    };
    for k in 0..tokens.len() {
        let before = k.checked_sub(1).and_then(|p| tokens.get(p)).copied();
        let after = tokens.get(k + 1).copied();
        let Some(token) = tokens.get_mut(k) else {
            continue;
        };
        let prev_end = word_heard(before.as_ref()).map(|(_, e)| e);
        let lo = prev_end.unwrap_or(token.start);
        let hi = match (token.kind, token.heard) {
            (TokenKind::Word, Some((s, _))) => s,
            _ => token.end,
        };
        // The first edit point past `start` and the low end, which is open
        // when no word comes before, and whether it is inside the high end.
        let low = lo.min(hi);
        let first =
            edits.partition_point(|e| *e <= start || *e < low || (prev_end.is_none() && *e == low));
        token.edit_before = before.is_some() && edits.get(first).is_some_and(|e| *e <= hi.max(lo));
        let next_start = word_heard(after.as_ref()).map(|(s, _)| s);
        token.tight =
            prev_end.is_some_and(|e| token.start < e) || next_start.is_some_and(|s| token.end > s);
    }
}

/// Put in the words a delete or a lift took out of `comp`'s top-level rows:
/// between two clips of one row that play the same footage unretimed, where
/// the later starts further into the source than the earlier ends, the words
/// whose middle lies in what was skipped, each at the edit point. The head
/// and tail trims of clips are not counted.
pub(super) fn add_removed(doc: &Document, comp: &Composition, tokens: &mut Vec<Token>) {
    // Each word's edit point, its token, and where it would be said had the
    // earlier clip run on.
    let mut found: Vec<(Rational, Token, (Rational, Rational))> = Vec::new();
    for (_, layer) in heard_layers(comp) {
        let LayerKind::Sequence { clips } = &layer.kind else {
            continue;
        };
        let mut order: Vec<_> = clips.iter().filter(|c| c.audible).collect();
        order.sort_by_key(|c| c.place_start);
        for pair in order.windows(2) {
            let [a, b] = pair else {
                continue;
            };
            let (ClipSource::Footage(item), ClipSource::Footage(later)) = (a.source, b.source)
            else {
                continue;
            };
            let plain = a.retime.is_none() && b.retime.is_none();
            if item != later
                || !plain
                || b.source_in <= a.source_out
                || b.place_start < a.place_end()
            {
                continue;
            }
            let Some(transcript) = doc.transcripts.get(&item) else {
                continue;
            };
            if !a.audio_streams.places().contains(&transcript.stream) {
                continue;
            }
            let Ok(at) = a.place_end().checked_add(layer.start_offset.0) else {
                continue;
            };
            let range = transcript.words_between(SourceTime(a.source_out), SourceTime(b.source_in));
            for index in range {
                let Some(word) = transcript.words.get(index) else {
                    continue;
                };
                let mid = middle(word.start.0, word.end.0);
                if mid < a.source_out || mid >= b.source_in {
                    continue;
                }
                let run_on =
                    |s: Rational| s.checked_sub(a.source_out).and_then(|d| at.checked_add(d));
                let (Ok(from), Ok(to)) = (run_on(word.start.0), run_on(word.end.0)) else {
                    continue;
                };
                found.push((
                    at,
                    Token {
                        word: Some(WordAt { item, index }),
                        row: Some(layer.id),
                        clip: Some(a.id),
                        edit_before: true,
                        ..blank(TokenKind::Removed, at, at)
                    },
                    (from, to),
                ));
            }
        }
    }
    // At one edit point, in the order they were said.
    found.sort_by_key(|(at, _, span)| (*at, span.0));
    // Each goes in at the token edge nearest its edit point, so the tiling
    // holds. The edges are in order, and so is `placed` by edge.
    let edges: Vec<Rational> = tokens
        .iter()
        .map(|t| t.start)
        .chain(tokens.last().map(|t| t.end))
        .collect();
    let mut placed: Vec<(usize, Token, (Rational, Rational))> = Vec::with_capacity(found.len());
    for (at, mut token, span) in found {
        let distance = |e: &Rational| {
            let d = e.checked_sub(at).unwrap_or(Rational::ZERO);
            d.max(d.checked_neg().unwrap_or(d))
        };
        let after = edges.partition_point(|e| *e < at);
        let below = after.checked_sub(1).and_then(|i| edges.get(i));
        let Some(e) = below
            .into_iter()
            .chain(edges.get(after))
            .copied()
            .min_by_key(distance)
        else {
            continue;
        };
        let k = edges.partition_point(|x| *x < e);
        token.start = e;
        token.end = e;
        // Two microphones hold the same word: one of them is enough.
        let said = |t: &Token| {
            let w = t.word?;
            let word = doc.transcripts.get(&w.item)?.words.get(w.index)?;
            Some((fillers::fold(&word.text), word.confidence))
        };
        let mine = said(&token);
        let same_edge = placed.partition_point(|(at, _, _)| *at < k);
        let twin = placed
            .get(same_edge..)
            .unwrap_or_default()
            .iter()
            .position(|(_, other, (a, b))| {
                other.row != token.row
                    && *a < span.1
                    && span.0 < *b
                    && said(other).map(|s| s.0) == mine.as_ref().map(|m| m.0.clone())
            })
            .map(|i| same_edge + i);
        match (twin, mine) {
            (Some(i), Some((_, sure))) => {
                if let Some(slot) = placed.get_mut(i) {
                    if said(&slot.1).is_some_and(|(_, other)| sure > other) {
                        slot.1 = token;
                        slot.2 = span;
                    }
                }
            }
            _ => placed.push((k, token, span)),
        }
    }
    let mut out = Vec::with_capacity(tokens.len() + placed.len());
    let mut placed = placed.into_iter().peekable();
    for (i, token) in std::mem::take(tokens).into_iter().enumerate() {
        while let Some((_, removed, _)) = placed.next_if(|(k, _, _)| *k <= i) {
            out.push(removed);
        }
        out.push(token);
    }
    out.extend(placed.map(|(_, removed, _)| removed));
    *tokens = out;
}

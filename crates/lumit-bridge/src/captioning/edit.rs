//! The Captions panel's edits: text, timing, splitting, joining, limits,
//! find and replace, and speakers. Each is one op and one undo step.

use super::*;

/// Words for new text in a cue, keeping the times of the words that stayed the
/// same and spreading the new ones over the gaps they fill.
fn retimed(old: &Cue, text: &str) -> Cue {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let mut words: Vec<String> = Vec::new();
    let mut breaks = Vec::new();
    for line in &lines {
        if !words.is_empty() {
            breaks.push(words.len());
        }
        words.extend(line.split_whitespace().map(str::to_owned));
    }
    // The longest run of words both versions share, in order.
    let (n, m) = (old.words.len(), words.len());
    let mut table = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[i][j] = if old.words[i].text == words[j] {
                table[i + 1][j + 1] + 1
            } else {
                table[i + 1][j].max(table[i][j + 1])
            };
        }
    }
    let mut same: Vec<Option<usize>> = vec![None; m];
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if old.words[i].text == words[j] {
            same[j] = Some(i);
            i += 1;
            j += 1;
        } else if table[i + 1][j] >= table[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    let speaker = old.words.first().and_then(|w| w.speaker);
    let mut out: Vec<CueWord> = Vec::with_capacity(m);
    let mut k = 0;
    while k < m {
        if let Some(i) = same[k] {
            let mut word = old.words[i].clone();
            word.text = words[k].clone();
            out.push(word);
            k += 1;
            continue;
        }
        // A run of new words, between the kept word before and the one after.
        let run_end = (k..m).find(|&r| same[r].is_some()).unwrap_or(m);
        let from = out.last().map_or(old.start, |w| w.end);
        let to = same
            .get(run_end)
            .copied()
            .flatten()
            .map_or(old.end, |i| old.words[i].start)
            .max(from);
        let weights: Vec<i64> = words[k..run_end]
            .iter()
            .map(|w| i64::try_from(w.chars().count()).unwrap_or(1) + 1)
            .collect();
        let total: i64 = weights.iter().sum::<i64>().max(1);
        let span = to.checked_sub(from).unwrap_or(Rational::ZERO);
        let at = |w: i64| {
            Rational::new(w, total)
                .and_then(|share| span.checked_mul(share))
                .and_then(|offset| from.checked_add(offset))
                .unwrap_or(from)
        };
        let mut gone = 0;
        for (r, weight) in (k..run_end).zip(&weights) {
            out.push(CueWord {
                text: words[r].clone(),
                start: at(gone),
                end: at(gone + weight),
                speaker,
                emphasis: false,
            });
            gone += weight;
        }
        k = run_end;
    }
    Cue {
        words: out,
        breaks,
        ..old.clone()
    }
}

pub(crate) fn set_text(
    layer: &LayerReference,
    index: u32,
    text: String,
) -> Result<(), BridgeError> {
    let held = held(layer)?;
    let old = cue(&held, index)?;
    if text.trim().is_empty() {
        return commit_cues(layer, index as usize, 1, Vec::new());
    }
    let mut next = retimed(&old, &text);
    if next.breaks.is_empty() {
        next = next.rebroken(&held.captions.limits);
    }
    commit_cues(layer, index as usize, 1, vec![next])
}

pub(crate) fn set_span(
    layer: &LayerReference,
    index: u32,
    start: crate::api::effect::BridgeRational,
    end: crate::api::effect::BridgeRational,
) -> Result<(), BridgeError> {
    let held = held(layer)?;
    let mut next = cue(&held, index)?;
    let (start, end) = (time_of(start)?.to_f64(), time_of(end)?.to_f64());
    // Only the start is turned into the cue's clock: an end on an edit point
    // belongs to the next clip. The length follows at the speed there.
    let (at, speed) = clock_at(&held, next.item, start).ok_or(BridgeError::InvalidTime)?;
    let length = Rational::from_f64_on_grid((end - start) * speed.abs(), 1000)
        .map_err(|_| BridgeError::InvalidTime)?;
    if length <= Rational::ZERO {
        return Err(BridgeError::InvalidTime);
    }
    let old = next.end.checked_sub(next.start).unwrap_or(Rational::ZERO);
    // Moved, rather than trimmed, when it keeps its length to a millisecond.
    let moved = (length.to_f64() - old.to_f64()).abs() < 0.0015;
    let length = if moved { old } else { length };
    // Played backwards, the moment shown first is the cue's end.
    let (start, end) = if speed < 0.0 {
        (at.checked_sub(length), Ok(at))
    } else {
        (Ok(at), at.checked_add(length))
    };
    let (start, end) = (start.unwrap_or(at), end.unwrap_or(at));
    let shift = start.checked_sub(next.start).unwrap_or(Rational::ZERO);
    // Never into a neighbour of the same footage.
    let (low, high) = neighbours(&held.captions, index as usize, next.item);
    next.start = start.max(low);
    next.end = end.min(high).max(next.start);
    for word in &mut next.words {
        if moved {
            word.start = word.start.checked_add(shift).unwrap_or(word.start);
            word.end = word.end.checked_add(shift).unwrap_or(word.end);
        }
        word.start = word.start.clamp(next.start, next.end);
        word.end = word.end.clamp(word.start, next.end);
    }
    commit_cues(layer, index as usize, 1, vec![next])
}

/// How far a cue of `item` may reach before it meets the cues either side of
/// it on the same clock.
fn neighbours(captions: &Captions, index: usize, item: Option<Uuid>) -> (Rational, Rational) {
    let me = captions.cues.get(index);
    let mut low = Rational::ZERO;
    let mut high = Rational::new(i64::MAX / 2, 1).unwrap_or(Rational::ONE);
    for (i, other) in captions.cues.iter().enumerate() {
        if i == index || other.item != item {
            continue;
        }
        if let Some(me) = me {
            if other.end <= me.start {
                low = low.max(other.end);
            } else if other.start >= me.end {
                high = high.min(other.start);
            }
        }
    }
    (low, high)
}

pub(crate) fn split(layer: &LayerReference, index: u32, at_word: u32) -> Result<(), BridgeError> {
    let held = held(layer)?;
    let old = cue(&held, index)?;
    let k = at_word as usize;
    if k == 0 || k >= old.words.len() {
        return Err(BridgeError::NothingToSplit);
    }
    let limits = &held.captions.limits;
    let first = Cue {
        end: old.words[k].start.max(old.start),
        words: old.words[..k].to_vec(),
        breaks: Vec::new(),
        ..old.clone()
    }
    .rebroken(limits);
    let second = Cue {
        start: old.words[k].start,
        words: old.words[k..].to_vec(),
        breaks: Vec::new(),
        label: None,
        ..old.clone()
    }
    .rebroken(limits);
    commit_cues(layer, index as usize, 1, vec![first, second])
}

pub(crate) fn split_at(
    layer: &LayerReference,
    at: crate::api::effect::BridgeRational,
) -> Result<(), BridgeError> {
    let held = held(layer)?;
    let t = time_of(at)?.to_f64();
    let context = lumit_core::expression::ExpressionContext {
        document: Arc::clone(&held.doc),
        comp: Some(layer.comp_id),
        layer: Some(layer.layer_id),
        comp_time: t,
        current_depth: 0,
        inputs: None,
    };
    let shown = show::at(&held.captions, &context);
    let index = shown.cue.ok_or(BridgeError::NothingToSplit)?;
    // The first word that starts at or after the playhead begins the new one.
    let lt = t - held.layer.start_offset.0.to_f64();
    let after = shown
        .words
        .iter()
        .position(|w| w.start >= lt)
        .ok_or(BridgeError::NothingToSplit)?;
    let k = *shown.places.get(after).ok_or(BridgeError::NothingToSplit)?;
    split(
        layer,
        u32::try_from(index).unwrap_or(u32::MAX),
        u32::try_from(k).unwrap_or(0),
    )
}

pub(crate) fn merge(layer: &LayerReference, first: u32, count: u32) -> Result<(), BridgeError> {
    let held = held(layer)?;
    let (from, to) = (first as usize, first as usize + count as usize);
    let run = held
        .captions
        .cues
        .get(from..to)
        .ok_or(BridgeError::InvalidItem)?;
    if run.len() < 2 || run.iter().any(|c| c.item != run[0].item) {
        return Err(BridgeError::NothingToSplit);
    }
    let mut words: Vec<CueWord> = run.iter().flat_map(|c| c.words.iter().cloned()).collect();
    words.sort_by_key(|w| w.start);
    let merged = Cue {
        start: run.iter().map(|c| c.start).min().unwrap_or(run[0].start),
        end: run.iter().map(|c| c.end).max().unwrap_or(run[0].end),
        words,
        breaks: Vec::new(),
        ..run[0].clone()
    }
    .rebroken(&held.captions.limits);
    commit_cues(layer, from, run.len(), vec![merged])
}

pub(crate) fn add(
    layer: &LayerReference,
    at: crate::api::effect::BridgeRational,
    text: String,
) -> Result<u32, BridgeError> {
    let held = held(layer)?;
    let t = time_of(at)?.to_f64();
    let source = source_of(&held.comp, &held.captions);
    let item = source
        .and_then(|s| show::heard_at(&held.doc, s, t, &|_, _| true))
        .map(|heard| heard.item);
    if source.is_some() && item.is_none() {
        return Err(BridgeError::NoClipThere);
    }
    let (start, _) = clock_at(&held, item, t).ok_or(BridgeError::InvalidTime)?;
    let typed = Rational::new(TYPED_SECONDS, 1).unwrap_or(Rational::ONE);
    let (_, high) = neighbours_of_new(&held.captions, item, start);
    let end = start.checked_add(typed).unwrap_or(start).min(high);
    if end <= start {
        return Err(BridgeError::InvalidTime);
    }
    let blank = Cue {
        start,
        end,
        words: Vec::new(),
        breaks: Vec::new(),
        label: None,
        dashes: false,
        item,
        offset: None,
        extra: serde_json::Map::new(),
    };
    let mut fresh = retimed(&blank, if text.trim().is_empty() { "..." } else { &text });
    fresh = fresh.rebroken(&held.captions.limits);
    // After every cue that starts before it, so the list keeps time order.
    let index = held
        .captions
        .cues
        .iter()
        .position(|c| c.item == item && c.start > start)
        .unwrap_or(held.captions.cues.len());
    commit_cues(layer, index, 0, vec![fresh])?;
    Ok(u32::try_from(index).unwrap_or(u32::MAX))
}

/// The room a new cue of `item` starting at `start` has before the next.
fn neighbours_of_new(
    captions: &Captions,
    item: Option<Uuid>,
    start: Rational,
) -> (Rational, Rational) {
    let mut low = Rational::ZERO;
    let mut high = Rational::new(i64::MAX / 2, 1).unwrap_or(Rational::ONE);
    for other in captions.cues.iter().filter(|c| c.item == item) {
        if other.end <= start {
            low = low.max(other.end);
        } else if other.start >= start {
            high = high.min(other.start);
        } else {
            // Inside another cue: no room at all.
            high = start;
        }
    }
    (low, high)
}

/// The cues at `indices` below `len` as runs of neighbours, each its first
/// index and its length, the last run first so each index still names the
/// cue it named.
fn runs(mut indices: Vec<u32>, len: usize) -> Vec<(usize, usize)> {
    indices.sort_unstable();
    indices.dedup();
    let mut out: Vec<(usize, usize)> = Vec::new();
    for i in indices.into_iter().map(|i| i as usize).filter(|&i| i < len) {
        match out.last_mut() {
            Some((at, n)) if *at + *n == i => *n += 1,
            _ => out.push((i, 1)),
        }
    }
    out.reverse();
    out
}

pub(crate) fn remove(layer: &LayerReference, cues: Vec<u32>) -> Result<(), BridgeError> {
    let held = held(layer)?;
    // One splice a run, so the journal carries the cues that went and
    // nothing more.
    let ops: Vec<Op> = runs(cues, held.captions.cues.len())
        .into_iter()
        .map(|(at, remove)| Op::EditCaptions {
            comp: layer.comp_id,
            layer: layer.layer_id,
            edit: Box::new(CaptionsEdit::Cues {
                at,
                remove,
                cues: Vec::new(),
            }),
        })
        .collect();
    if ops.is_empty() {
        return Ok(());
    }
    layer.commit(Op::Batch { ops })
}

pub(crate) fn set_breaks(
    layer: &LayerReference,
    index: u32,
    breaks: Vec<u32>,
) -> Result<(), BridgeError> {
    let held = held(layer)?;
    let mut next = cue(&held, index)?;
    let mut breaks: Vec<usize> = breaks
        .into_iter()
        .map(|b| b as usize)
        .filter(|&b| b > 0 && b < next.words.len())
        .collect();
    breaks.sort_unstable();
    breaks.dedup();
    next.breaks = breaks;
    commit_cues(layer, index as usize, 1, vec![next])
}

pub(crate) fn set_offset(
    layer: &LayerReference,
    index: u32,
    x: f64,
    y: f64,
) -> Result<(), BridgeError> {
    let held = held(layer)?;
    let mut next = cue(&held, index)?;
    let moved = x.is_finite() && y.is_finite() && (x != 0.0 || y != 0.0);
    next.offset = moved.then_some([x, y]);
    commit_cues(layer, index as usize, 1, vec![next])
}

/// Raise captions to the top of the picture, or put them back down, in one
/// undo step. Raised, each moves up by the distance from where the layer puts
/// its last line to the same margin from the top.
pub(crate) fn raise(
    layer: &LayerReference,
    cues: Vec<u32>,
    raised: bool,
) -> Result<(), BridgeError> {
    let held = held(layer)?;
    let LayerKind::Text { document } = &held.layer.kind else {
        return Err(BridgeError::NotCaptions);
    };
    // The caption box's bottom sits at the layer's position; its top is a box
    // height above. The same margin it keeps from the bottom, kept from the top.
    // The move is in the layer's own pixels, so a scaled layer is allowed for.
    let transform = &held.layer.transform;
    let scale = (transform.scale_y.value_at(0.0) / 100.0).abs().max(0.01);
    let bottom = transform.position_y.value_at(0.0);
    let box_height = f64::from(document.paragraph.box_height).max(document.size * 1.2) * scale;
    let margin = (f64::from(held.comp.height) - bottom).max(0.0);
    let up = (margin + box_height - bottom) / scale;
    let offset = (raised && up != 0.0).then_some([0.0, up]);
    let len = held.captions.cues.len();
    if cues.iter().any(|&i| i as usize >= len) {
        return Err(BridgeError::InvalidItem);
    }
    let runs = runs(cues, len);
    // One run is one splice; scattered ones are one change to the whole list
    // rather than a check of every cue for each.
    let moved = |cue: &Cue| Cue {
        offset,
        ..cue.clone()
    };
    match runs.as_slice() {
        [] => Ok(()),
        &[(at, n)] => {
            let run = held.captions.cues.get(at..at + n).unwrap_or_default();
            commit_cues(layer, at, n, run.iter().map(moved).collect())
        }
        _ => {
            let mut next = (*held.captions).clone();
            for &(at, n) in &runs {
                for cue in next.cues.get_mut(at..at + n).into_iter().flatten() {
                    cue.offset = offset;
                }
            }
            commit_all(layer, next)
        }
    }
}

pub(crate) fn set_emphasis(
    layer: &LayerReference,
    index: u32,
    word: u32,
    on: bool,
) -> Result<(), BridgeError> {
    let held = held(layer)?;
    let mut next = cue(&held, index)?;
    let word = next
        .words
        .get_mut(word as usize)
        .ok_or(BridgeError::InvalidItem)?;
    word.emphasis = on;
    commit_cues(layer, index as usize, 1, vec![next])
}

pub(crate) fn set_limits(
    layer: &LayerReference,
    limits: BridgeCaptionLimits,
    rebuild: bool,
) -> Result<(), BridgeError> {
    let held = held(layer)?;
    let mut next = (*held.captions).clone();
    next.limits = limits_in(&held.comp, &limits);
    if rebuild {
        next.cues = rebuilt(&next);
    }
    commit_all(layer, next)
}

/// Every cue cut again from its words under the captions' own limits, one
/// footage item at a time.
fn rebuilt(captions: &Captions) -> Vec<Cue> {
    let names = |id: Uuid| {
        captions
            .speakers
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.name.clone())
    };
    let mut items: Vec<Option<Uuid>> = Vec::new();
    for cue in &captions.cues {
        if !items.contains(&cue.item) {
            items.push(cue.item);
        }
    }
    let mut out = Vec::new();
    for item in items {
        let mut words: Vec<CueWord> = captions
            .cues
            .iter()
            .filter(|c| c.item == item)
            .flat_map(|c| c.words.iter().cloned())
            .collect();
        words.sort_by_key(|w| w.start);
        for mut cue in cues::build(&words, &captions.limits, &names) {
            cue.item = item;
            out.push(cue);
        }
    }
    out
}

pub(crate) fn set_language(layer: &LayerReference, language: String) -> Result<(), BridgeError> {
    let held = held(layer)?;
    let mut next = (*held.captions).clone();
    next.language = language.trim().to_owned();
    commit_all(layer, next)
}

pub(crate) fn replace(
    layer: &LayerReference,
    find: String,
    with: String,
    match_case: bool,
    whole_word: bool,
) -> Result<u32, BridgeError> {
    if find.is_empty() {
        return Ok(0);
    }
    let held = held(layer)?;
    let mut next = (*held.captions).clone();
    let mut count = 0u32;
    for cue in &mut next.cues {
        let text = cue.lines().join("\n");
        let (changed, n) = replaced(&text, &find, &with, match_case, whole_word);
        if n == 0 {
            continue;
        }
        count += n;
        let label = cue.label.clone();
        let dashes = cue.dashes;
        let mut fresh = retimed(cue, &strip_marks(&changed, label.as_deref(), dashes));
        fresh.label = label;
        fresh.dashes = dashes;
        *cue = fresh;
    }
    if count > 0 {
        commit_all(layer, next)?;
    }
    Ok(count)
}

/// A cue's text without the label and dashes [`Cue::lines`] puts in.
fn strip_marks(text: &str, label: Option<&str>, dashes: bool) -> String {
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    if let (Some(label), Some(first)) = (label.filter(|l| !l.is_empty()), lines.first_mut()) {
        if let Some(rest) = first.strip_prefix(label) {
            *first = rest.trim_start().to_owned();
        }
    }
    if dashes {
        for line in &mut lines {
            if let Some(rest) = line.strip_prefix("- ") {
                *line = rest.to_owned();
            }
        }
    }
    lines.join("\n")
}

/// `text` with every `find` swapped for `with`, and how many there were.
fn replaced(
    text: &str,
    find: &str,
    with: &str,
    match_case: bool,
    whole_word: bool,
) -> (String, u32) {
    let hay: Vec<char> = text.chars().collect();
    let needle: Vec<char> = find.chars().collect();
    let same = |a: char, b: char| {
        if match_case {
            a == b
        } else {
            a.to_lowercase().eq(b.to_lowercase())
        }
    };
    let mut out = String::with_capacity(text.len());
    let mut count = 0;
    let mut i = 0;
    while i < hay.len() {
        let fits = i + needle.len() <= hay.len()
            && needle.iter().enumerate().all(|(k, c)| same(hay[i + k], *c));
        let bounded = !whole_word
            || ((i == 0 || !hay[i - 1].is_alphanumeric())
                && hay
                    .get(i + needle.len())
                    .is_none_or(|c| !c.is_alphanumeric()));
        if fits && bounded && !needle.is_empty() {
            out.push_str(with);
            count += 1;
            i += needle.len();
        } else {
            out.push(hay[i]);
            i += 1;
        }
    }
    (out, count)
}

// ---- Speakers ----------------------------------------------------------------

/// The edits that rename `speaker` on every caption layer that has them, and
/// put the new name on their labels.
pub(crate) fn renamed(doc: &Document, speaker: Uuid, name: &str) -> Vec<Op> {
    let mut ops = Vec::new();
    for item in &doc.items {
        let lumit_core::model::ProjectItem::Composition(comp) = item else {
            continue;
        };
        for layer in &comp.layers {
            let LayerKind::Text { document } = &layer.kind else {
                continue;
            };
            let Some(captions) = document.captions.as_ref() else {
                continue;
            };
            let Some(old) = captions.speakers.iter().find(|s| s.id == speaker) else {
                continue;
            };
            let (old_label, new_label) = (format!("{}:", old.name), format!("{name}:"));
            let mut next = (**captions).clone();
            for s in &mut next.speakers {
                if s.id == speaker {
                    s.name = name.to_owned();
                }
            }
            for cue in &mut next.cues {
                if cue.label.as_deref() == Some(old_label.as_str())
                    && cue.words.first().and_then(|w| w.speaker) == Some(speaker)
                {
                    cue.label = Some(new_label.clone());
                }
            }
            ops.push(Op::EditCaptions {
                comp: comp.id,
                layer: layer.id,
                edit: Box::new(CaptionsEdit::Set(Some(Arc::new(next)))),
            });
        }
    }
    ops
}

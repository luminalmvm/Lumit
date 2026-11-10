//! What a caption layer shows at one moment, and where each word sits in time.
//!
//! # In plain terms
//!
//! Captions made from a layer keep their words in the source time of the
//! footage they were heard in. To show them, the moment on screen is turned
//! into "which footage, and how far in" through the source layer's clips,
//! trims and Retime, the caption being said there is found, and each of its
//! words is carried back the same way to the moment it is heard in the edit. A
//! word whose moment is not in the edit any more, because that stretch was cut
//! out, is left out of the caption.
//!
//! Captions on their own clock skip all of that: their times are the caption
//! layer's own.
//!
//! # Thread role and contract
//!
//! Pure reads of a document snapshot, on whichever thread renders or keys a
//! frame. No IO, no clocks. The same snapshot and moment always give the same
//! answer.

use std::{borrow::Cow, sync::Arc};

use uuid::Uuid;

use crate::{
    expression::ExpressionContext,
    model::{Composition, Document, Layer, LayerKind, TextDocument},
    sequence::ClipSource,
    text::SpokenWord,
};

use super::{cues::Cue, Captions};

/// How far either side of the moment on screen a caption's other words are
/// always looked for, in seconds. Further at a slower speed, and anywhere in
/// the stretch the caption itself shows over.
const WINDOW: f64 = 12.0;

/// What a caption layer shows at one moment.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Shown {
    /// The caption's text, its lines on lines of their own. Empty between
    /// captions.
    pub text: String,
    /// Each word's characters in `text`, and when it is said in the caption
    /// layer's own seconds.
    pub words: Vec<SpokenWord>,
    /// Each of `words`' place in the cue's own words.
    pub places: Vec<usize>,
    /// Which cue it is, by its place in the list.
    pub cue: Option<usize>,
    /// How far the cue is moved from where the layer puts it, in the layer's
    /// own pixels.
    pub offset: [f64; 2],
}

/// What a Text layer says at the moment `context` names, when each word of
/// it is said, and how far a caption is moved off its place: a caption
/// layer's caption, or any other layer's own text, whose words have no times
/// and which is never moved.
#[must_use]
pub fn spoken(
    document: &TextDocument,
    context: Arc<ExpressionContext>,
) -> (Cow<'_, str>, Vec<SpokenWord>, [f64; 2]) {
    match &document.captions {
        Some(captions) => {
            let shown = at(captions, &context);
            (Cow::Owned(shown.text), shown.words, shown.offset)
        }
        None => (document.resolved_text(context), Vec::new(), [0.0; 2]),
    }
}

/// How far the caption `layer` shows at the comp time `context` names is
/// moved from where the layer puts it, in the layer's own pixels: nought for
/// any other layer. The same clock [`spoken`] reads, so the words and their
/// place agree.
#[must_use]
pub fn moved(layer: &Layer, context: &ExpressionContext) -> (f64, f64) {
    let LayerKind::Text { document } = &layer.kind else {
        return (0.0, 0.0);
    };
    let Some(captions) = document.captions.as_deref() else {
        return (0.0, 0.0);
    };
    if captions.cues.iter().all(|c| c.offset.is_none()) {
        return (0.0, 0.0);
    }
    let moment = ExpressionContext {
        layer: Some(layer.id),
        ..context.clone()
    };
    let [x, y] = at(captions, &moment).offset;
    (x, y)
}

/// What `captions` show at the moment `context` names: its comp time, on the
/// caption layer it names, in the document it carries.
#[must_use]
pub fn at(captions: &Captions, context: &ExpressionContext) -> Shown {
    let t = context.comp_time;
    let comp = context.comp.and_then(|id| context.document.comp(id));
    let offset = comp
        .and_then(|c| c.layers.iter().find(|l| Some(l.id) == context.layer))
        .map_or(0.0, |l| l.start_offset.0.to_f64());
    let source = captions
        .source
        .and_then(|id| comp.and_then(|c| c.layers.iter().find(|l| l.id == id)));
    match source {
        Some(layer) => linked(&context.document, captions, layer, t, offset),
        // Unlinked, or the layer they were made from has gone: the cues on
        // this layer's own clock show by it.
        None => own(captions, t - offset),
    }
}

/// Captions on the caption layer's own clock, at its time `lt`. A cue kept in
/// a footage item's time whose layer has gone shows nothing.
fn own(captions: &Captions, lt: f64) -> Shown {
    let Some((index, cue)) = captions.cues.iter().enumerate().find(|(_, c)| {
        (captions.source.is_none() || c.item.is_none())
            && c.start.to_f64() <= lt
            && lt < c.end.to_f64()
    }) else {
        return Shown::default();
    };
    compose(index, cue, |start, end| Some((start, end)))
}

/// Captions linked to `layer`, at comp time `t`, shown on a layer whose start
/// offset is `offset`.
fn linked(doc: &Document, captions: &Captions, layer: &Layer, t: f64, offset: f64) -> Shown {
    let covering = |item: Uuid, s: f64| {
        captions
            .cues
            .iter()
            .enumerate()
            .find(|(_, c)| c.item == Some(item) && c.start.to_f64() <= s && s < c.end.to_f64())
    };
    let Some(heard) = heard_at(doc, layer, t, &|item, s| covering(item, s).is_some()) else {
        return Shown::default();
    };
    let Some((index, cue)) = covering(heard.item, heard.source) else {
        return Shown::default();
    };
    let item = heard.item;
    // A word is near when it is said within the caption's own length at the
    // speed here, or inside the stretch the caption shows over. So a slow
    // Retime, a freeze or a caption timed long keeps every word.
    let (start, end) = (cue.start.to_f64(), cue.end.to_f64());
    let reach = WINDOW.max((end - start) / heard.speed.abs().max(1e-6));
    let ends = place(doc, layer, item, start, t, 0).zip(place(doc, layer, item, end, t, 0));
    let shows = ends.map(|((a, _), (b, _))| (a.min(b), a.max(b)));
    compose(index, cue, |start, end| {
        let (at, speed) = place(doc, layer, item, start, t, 0)?;
        let inside = shows.is_some_and(|(a, b)| at >= a && at <= b);
        if !inside && (at - t).abs() > reach {
            return None;
        }
        let length = (end - start) / speed.abs().max(1e-6);
        Some((at - offset, at + length - offset))
    })
}

/// How many Precomp layers deep the words are looked for.
const DEPTH: u32 = 8;

/// What a layer plays at one moment: which footage, how far into it, and how
/// fast it runs there.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Heard {
    pub item: Uuid,
    pub source: f64,
    pub speed: f64,
}

/// Which footage `layer` plays at comp time `t`, how far into it and how fast,
/// taking only what `wanted` accepts of an item and a moment in it. Where
/// several play at once, in a dissolve or inside a Precomp layer, the first
/// accepted wins: the later clip, or the top layer inside.
#[must_use]
pub fn heard_at(
    doc: &Document,
    layer: &Layer,
    t: f64,
    wanted: &dyn Fn(Uuid, f64) -> bool,
) -> Option<Heard> {
    heard_in(doc, layer, t, wanted, 0)
}

fn heard_in(
    doc: &Document,
    layer: &Layer,
    t: f64,
    wanted: &dyn Fn(Uuid, f64) -> bool,
    depth: u32,
) -> Option<Heard> {
    use crate::retime::property_speed_at as speed;
    if depth > DEPTH || t < layer.in_point.0.to_f64() || t >= layer.out_point.0.to_f64() {
        return None;
    }
    let lt = t - layer.start_offset.0.to_f64();
    let heard = |item: Uuid, source: f64, speed: f64| {
        wanted(item, source).then_some(Heard {
            item,
            source,
            speed,
        })
    };
    match &layer.kind {
        LayerKind::Footage { item } => heard(
            *item,
            layer.source_time_at(lt),
            speed(layer.retime.as_ref(), lt),
        ),
        // In a dissolve the later clip is the one coming in.
        LayerKind::Sequence { clips } => {
            clips
                .iter()
                .rev()
                .filter(|c| c.contains(lt))
                .find_map(|clip| match clip.source {
                    ClipSource::Footage(item) => heard(
                        item,
                        clip.source_time(lt),
                        speed(clip.retime.as_ref(), lt - clip.place_start.to_f64()),
                    ),
                    // A multicam clip is heard as its angle's own file. Any
                    // other clip of a comp is not followed.
                    ClipSource::Comp(_) => {
                        let at = crate::time::Rational::from_f64_on_grid(lt, 1000).ok()?;
                        let shown = crate::multicam::heard_at(doc, clip, at)?;
                        heard(
                            shown.item,
                            shown.source_time.to_f64(),
                            speed(clip.retime.as_ref(), lt - clip.place_start.to_f64()),
                        )
                    }
                    ClipSource::Solid(_) => None,
                })
        }
        LayerKind::Precomp { comp } => {
            let nested = doc.comp(*comp)?;
            let inner = crate::model::nested_source_time(layer, nested, lt);
            let outer = speed(layer.retime.as_ref(), lt);
            nested
                .layers
                .iter()
                .filter(|l| l.switches.audible)
                .find_map(|l| heard_in(doc, l, inner, wanted, depth + 1))
                .map(|h| Heard {
                    speed: h.speed * outer,
                    ..h
                })
        }
        _ => None,
    }
}

/// The comp time nearest `t` at which `layer` plays moment `s` of `item`, and
/// how fast it plays there, or `None` when that moment is not in the edit.
fn place(
    doc: &Document,
    layer: &Layer,
    item: Uuid,
    s: f64,
    t: f64,
    depth: u32,
) -> Option<(f64, f64)> {
    if depth > DEPTH {
        return None;
    }
    let offset = layer.start_offset.0.to_f64();
    let (from, to) = (layer.in_point.0.to_f64(), layer.out_point.0.to_f64());
    let near = |at: f64| at >= from && at < to;
    // A layer's own clock to the moment it shows, and back: through its
    // Retime when it has one.
    let outer = |inner: f64| -> Option<f64> {
        match &layer.retime {
            None => Some(inner),
            Some(map) => inverse(map, from - offset, to - offset, inner, t - offset),
        }
    };
    match &layer.kind {
        LayerKind::Footage { item: own } if *own == item => {
            let lt = outer(s)?;
            let speed = crate::retime::property_speed_at(layer.retime.as_ref(), lt);
            let at = offset + lt;
            near(at).then_some((at, speed))
        }
        LayerKind::Sequence { clips } => clips
            .iter()
            .filter(|c| c.source == ClipSource::Footage(item))
            .filter_map(|clip| {
                let place_start = clip.place_start.to_f64();
                let length = clip.place_duration.to_f64();
                let local = match &clip.retime {
                    None => {
                        let (a, b) = (clip.source_in.to_f64(), clip.source_out.to_f64());
                        (s >= a && s <= b).then_some(s - a)?
                    }
                    Some(map) => inverse(map, 0.0, length, s, t - offset - place_start)?,
                };
                let speed = crate::retime::property_speed_at(clip.retime.as_ref(), local);
                let at = offset + place_start + local;
                (near(at) && local <= length).then_some((at, speed))
            })
            .min_by(|a, b| (a.0 - t).abs().total_cmp(&(b.0 - t).abs())),
        LayerKind::Precomp { comp } => {
            let nested = doc.comp(*comp)?;
            let inner_t = crate::model::nested_source_time(layer, nested, t - offset);
            let (inner, speed) = nested
                .layers
                .iter()
                .filter(|l| l.switches.audible)
                .filter_map(|l| place(doc, l, item, s, inner_t, depth + 1))
                .min_by(|a, b| (a.0 - inner_t).abs().total_cmp(&(b.0 - inner_t).abs()))?;
            let lt = outer(inner)?;
            let at = offset + lt;
            let outer_speed = crate::retime::property_speed_at(layer.retime.as_ref(), lt);
            near(at).then_some((at, speed * outer_speed))
        }
        _ => None,
    }
}

/// The input of `map` between `lo` and `hi` nearest `near` at which it gives
/// `target`, for a Retime map that may slow, stop or run backwards. Found by
/// stepping from key to key for a change of side and halving in on it.
fn inverse(map: &crate::anim::Property, lo: f64, hi: f64, target: f64, near: f64) -> Option<f64> {
    use crate::anim::{Animation, SideInterp};
    if !lo.is_finite() || !hi.is_finite() || hi <= lo {
        return None;
    }
    let mut best: Option<f64> = None;
    let Animation::Keyframed(keys) = &map.animation else {
        scan(map, (lo, hi), 64, target, near, &mut best);
        return best;
    };
    // A straight stretch between two keys runs one way, so one step finds
    // its root; a curved one, or either side of the keys, takes a few.
    let mut from = lo;
    for (i, key) in keys.iter().enumerate() {
        let to = key.time.to_f64().min(hi);
        if to <= from {
            continue;
        }
        let straight = i.checked_sub(1).and_then(|k| keys.get(k)).is_some_and(|b| {
            matches!(b.interp_out, SideInterp::Linear)
                && matches!(key.interp_in, SideInterp::Linear)
        });
        let steps = if straight { 1 } else { 8 };
        scan(map, (from, to), steps, target, near, &mut best);
        from = to;
    }
    if from < hi {
        scan(map, (from, hi), 8, target, near, &mut best);
    }
    best
}

/// [`inverse`] across one stretch in `steps` even steps, keeping in `best`
/// the root nearest `near`.
fn scan(
    map: &crate::anim::Property,
    (lo, hi): (f64, f64),
    steps: u32,
    target: f64,
    near: f64,
    best: &mut Option<f64>,
) {
    let map = |x: f64| map.value_at(x);
    let step = (hi - lo) / f64::from(steps);
    let mut a = lo;
    let mut fa = map(a) - target;
    for i in 1..=steps {
        let b = if i == steps {
            hi
        } else {
            lo + step * f64::from(i)
        };
        let fb = map(b) - target;
        let root = if fa == 0.0 {
            Some(a)
        } else if fa.signum() != fb.signum() && fb.is_finite() && fa.is_finite() {
            let (mut x0, mut x1, mut f0) = (a, b, fa);
            for _ in 0..40 {
                let mid = 0.5 * (x0 + x1);
                let fm = map(mid) - target;
                if fm == 0.0 {
                    (x0, x1) = (mid, mid);
                    break;
                }
                if fm.signum() == f0.signum() {
                    x0 = mid;
                    f0 = fm;
                } else {
                    x1 = mid;
                }
            }
            Some(0.5 * (x0 + x1))
        } else {
            None
        };
        let root = root.or_else(|| (i == steps && fb == 0.0).then_some(b));
        if let Some(root) = root {
            if best.is_none_or(|b| (root - near).abs() < (b - near).abs()) {
                *best = Some(root);
            }
        }
        a = b;
        fa = fb;
    }
}

/// The caption `cue` as shown, its words placed by `when`, which turns a
/// word's own start and end into the caption layer's seconds or leaves it out.
fn compose(index: usize, cue: &Cue, when: impl Fn(f64, f64) -> Option<(f64, f64)>) -> Shown {
    let kept: Vec<(usize, f64, f64)> = cue
        .words
        .iter()
        .enumerate()
        .filter_map(|(i, w)| {
            when(w.start.to_f64(), w.end.to_f64()).map(|(start, end)| (i, start, end))
        })
        .collect();
    if kept.is_empty() {
        return Shown::default();
    }
    let mut text = String::new();
    let mut count = 0usize;
    let mut push = |text: &mut String, piece: &str| {
        text.push_str(piece);
        count += piece.chars().count();
        count
    };
    if let Some(label) = cue.label.as_deref().filter(|l| !l.is_empty()) {
        push(&mut text, label);
        push(&mut text, " ");
    }
    let mut words = Vec::with_capacity(kept.len());
    let mut previous: Option<usize> = None;
    for &(i, start, end) in &kept {
        let new_line = previous.is_some_and(|p| cue.breaks.iter().any(|&b| b > p && b <= i));
        match previous {
            None => {
                if cue.dashes {
                    push(&mut text, "- ");
                }
            }
            Some(p) => {
                if new_line {
                    push(&mut text, "\n");
                    if cue.dashes {
                        push(&mut text, "- ");
                    }
                } else if super::cues::spaced(&cue.words[p].text, &cue.words[i].text) {
                    push(&mut text, " ");
                }
            }
        }
        let from = push(&mut text, "");
        let to = push(&mut text, &cue.words[i].text);
        words.push(SpokenWord {
            chars: from..to,
            start,
            end: end.max(start),
            emphasis: cue.words[i].emphasis,
        });
        previous = Some(i);
    }
    Shown {
        text,
        words,
        places: kept.iter().map(|&(i, _, _)| i).collect(),
        cue: Some(index),
        offset: cue.offset.unwrap_or_default(),
    }
}

/// Every caption a caption layer shows across `times`, in comp time, as cues
/// for a subtitle file: one for each run of moments showing the same caption
/// the same way, from its first moment to the moment after its last.
///
/// `times` are the moments to look at, in order, such as every frame of an
/// export; the moment after the last is `end`. `context` names the document,
/// the comp and the caption layer, and its own comp time is ignored. `stop`
/// is asked now and then whether to give up, a long export being hours of
/// moments; given up, nothing comes back.
#[must_use]
pub fn spans(
    captions: &Captions,
    context: &ExpressionContext,
    times: &[f64],
    end: f64,
    stop: &dyn Fn() -> bool,
) -> Vec<Cue> {
    let layer = context
        .comp
        .and_then(|id| context.document.comp(id))
        .and_then(|c| c.layers.iter().find(|l| Some(l.id) == context.layer));
    let (from, to, offset) = layer.map_or((f64::MIN, f64::MAX, 0.0), |l| {
        (
            l.in_point.0.to_f64(),
            l.out_point.0.to_f64(),
            l.start_offset.0.to_f64(),
        )
    });
    let mut out: Vec<(Shown, f64, f64)> = Vec::new();
    let mut moment = context.clone();
    for (i, &t) in times.iter().enumerate() {
        if i % 256 == 0 && stop() {
            return Vec::new();
        }
        let next = times.get(i + 1).copied().unwrap_or(end);
        let shown = if t >= from && t < to {
            moment.comp_time = t;
            at(captions, &moment)
        } else {
            Shown::default()
        };
        if shown.text.is_empty() {
            continue;
        }
        match out.last_mut() {
            Some((last, _, until))
                if *until == t && last.cue == shown.cue && last.text == shown.text =>
            {
                *until = next;
            }
            _ => out.push((shown, t, next)),
        }
    }
    out.into_iter()
        .filter_map(|(shown, start, end)| cue_in_comp_time(captions, &shown, start, end, offset))
        .collect()
}

/// One shown caption as a cue on the comp's clock.
fn cue_in_comp_time(
    captions: &Captions,
    shown: &Shown,
    start: f64,
    end: f64,
    offset: f64,
) -> Option<Cue> {
    let original = captions.cues.get(shown.cue?)?;
    let on_grid = |t: f64| crate::time::Rational::from_f64_on_grid(t.max(0.0), 1000).ok();
    let (start, end) = (on_grid(start)?, on_grid(end)?);
    let mut words = Vec::with_capacity(shown.words.len());
    let mut breaks = Vec::new();
    let chars: Vec<char> = shown.text.chars().collect();
    for (i, word) in shown.words.iter().enumerate() {
        if i > 0 {
            let before = shown.words[i - 1].chars.end;
            if chars
                .get(before..word.chars.start)
                .is_some_and(|gap| gap.contains(&'\n'))
            {
                breaks.push(i);
            }
        }
        let text: String = chars
            .get(word.chars.clone())
            .map(|c| c.iter().collect())
            .unwrap_or_default();
        let speaker = (shown.places.get(i))
            .and_then(|&k| original.words.get(k))
            .and_then(|w| w.speaker);
        let at = on_grid(word.start + offset)?.clamp(start, end);
        let until = on_grid(word.end + offset)?.clamp(at, end);
        words.push(super::cues::CueWord {
            text,
            start: at,
            end: until,
            speaker,
            emphasis: word.emphasis,
        });
    }
    Some(Cue {
        start,
        end,
        words,
        breaks,
        label: original.label.clone(),
        dashes: original.dashes,
        item: None,
        offset: None,
        extra: serde_json::Map::new(),
    })
}

/// Where each cue first shows, in comp time, from its start to its end; `None`
/// for a cue whose stretch is not in the edit at all. For listing and timing
/// captions in a panel, so it walks the source layer's clips once rather than
/// looking frame by frame.
#[must_use]
pub fn first_shown(
    doc: &Document,
    captions: &Captions,
    comp: &Composition,
    layer: &Layer,
) -> Vec<Option<(f64, f64)>> {
    let offset = layer.start_offset.0.to_f64();
    let source = captions
        .source
        .and_then(|id| comp.layers.iter().find(|l| l.id == id));
    let own = |c: &Cue| {
        (captions.source.is_none() || c.item.is_none())
            .then(|| (c.start.to_f64() + offset, c.end.to_f64() + offset))
    };
    let Some(source) = source else {
        return captions.cues.iter().map(own).collect();
    };
    let so = source.start_offset.0.to_f64();
    let (from, to) = (source.in_point.0.to_f64(), source.out_point.0.to_f64());
    let mut out: Vec<Option<(f64, f64)>> = vec![None; captions.cues.len()];
    let mut keep = |index: usize, start: f64, end: f64| {
        let (start, end) = (start.max(from), end.min(to));
        if end > start && out[index].is_none_or(|(s, _)| start < s) {
            out[index] = Some((start, end));
        }
    };
    match &source.kind {
        LayerKind::Footage { item } => {
            for (index, cue) in captions.cues.iter().enumerate() {
                if cue.item != Some(*item) {
                    continue;
                }
                let map = |s: f64| match &source.retime {
                    None => Some(so + s),
                    Some(retime) => {
                        inverse(retime, from - so, to - so, s, from - so).map(|lt| so + lt)
                    }
                };
                if let (Some(start), Some(end)) = (map(cue.start.to_f64()), map(cue.end.to_f64())) {
                    keep(index, start.min(end), start.max(end));
                }
            }
        }
        LayerKind::Sequence { clips } => {
            // Each item's cues in order of source time, so a clip finds the
            // ones it plays without looking at every cue.
            let mut by_item: std::collections::BTreeMap<Uuid, Vec<usize>> =
                std::collections::BTreeMap::new();
            for (index, cue) in captions.cues.iter().enumerate() {
                if let Some(item) = cue.item {
                    by_item.entry(item).or_default().push(index);
                }
            }
            for list in by_item.values_mut() {
                list.sort_by_key(|&i| captions.cues[i].start);
            }
            for clip in clips {
                let ClipSource::Footage(item) = clip.source else {
                    continue;
                };
                let Some(list) = by_item.get(&item) else {
                    continue;
                };
                let place_start = clip.place_start.to_f64();
                let length = clip.place_duration.to_f64();
                let (a, b) = (clip.source_in.to_f64(), clip.source_out.to_f64());
                let (low, high) = (a.min(b), a.max(b));
                let map = |s: f64| match &clip.retime {
                    None => Some(so + place_start + (s - a)),
                    Some(retime) => {
                        inverse(retime, 0.0, length, s, 0.0).map(|local| so + place_start + local)
                    }
                };
                let first = list.partition_point(|&i| captions.cues[i].end.to_f64() <= low);
                for &index in &list[first..] {
                    let cue = &captions.cues[index];
                    if cue.start.to_f64() >= high {
                        break;
                    }
                    let (s0, s1) = (cue.start.to_f64().max(low), cue.end.to_f64().min(high));
                    if let (Some(start), Some(end)) = (map(s0), map(s1)) {
                        keep(index, start.min(end), start.max(end));
                    }
                }
            }
        }
        LayerKind::Precomp { .. } => {
            // Inside a nested comp the edit can be anything, so each cue is
            // placed through the same walk the frames take, nearest the
            // layer's start.
            for (index, cue) in captions.cues.iter().enumerate() {
                let Some(item) = cue.item else {
                    continue;
                };
                let place = |s: f64| place(doc, source, item, s, from, 0);
                if let (Some((start, _)), Some((end, _))) =
                    (place(cue.start.to_f64()), place(cue.end.to_f64()))
                {
                    keep(index, start.min(end), start.max(end));
                }
            }
        }
        _ => return captions.cues.iter().map(own).collect(),
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;

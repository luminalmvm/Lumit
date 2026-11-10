//! A Text layer, two ways.
//!
//! **As shapes** (the default): each letter's outline as a group of shapes,
//! laid out by Lumit's own text code exactly where the picture puts the
//! ink, so every player draws the same letters without the font. What the
//! animators do to each letter is sampled per frame into that letter's
//! group transform and fill. Words an expression writes are written as one
//! set of letters per distinct text, each shown only while it is the text.
//!
//! **As text**: a Lottie text layer naming its font, which a player draws
//! only with that font installed, so the check says so.
//!
//! Runs on the export's thread.

use lumit_core::anim::Animation;
use lumit_core::model::TextDocument;
use lumit_core::shape::modifier::ShapeEntry;
use lumit_core::text::{GlyphXform, SelectorBasis, SelectorShape, TextAlign};
use lumit_text::TextBlock;
use serde_json::{json, Map, Value};

use super::shapes;
use super::value::{Class, Clock};
use crate::check::{arg, Feature, Place, Report};
use crate::fit::{self, tolerance};
use crate::keys::{Track, TrackKey};

/// The most distinct texts an expression's words are written as. Past it the
/// first text is held, and the check says so.
const MAX_TEXTS: usize = 120;

/// The text the layer shows over each stretch of its time: (words, the
/// layer time it starts showing them).
fn texts(c: &Clock, document: &TextDocument) -> Vec<(String, f64)> {
    let times = c.times();
    let first = times.first().copied().unwrap_or(0.0);
    if document.expression.is_none() {
        return vec![(document.text.clone(), first)];
    }
    c.baked.set(true);
    let mut out: Vec<(String, f64)> = Vec::new();
    for lt in times {
        let words = document.resolved_text(c.context(lt)).into_owned();
        if out.last().is_none_or(|(w, _)| *w != words) {
            out.push((words, lt));
        }
    }
    out
}

/// The layer's letters as Lottie shape groups, the top of the picture first.
pub fn shapes(
    c: &Clock,
    document: &TextDocument,
    report: &mut Report,
    place: &Place,
) -> Vec<Value> {
    let mut runs = texts(c, document);
    if runs.len() > MAX_TEXTS {
        runs.truncate(1);
        report.changed(
            place,
            "text_held",
            "The text is held at its first words".into(),
            Vec::new(),
        );
    }
    if runs.len() == 1 {
        let (words, start) = runs.swap_remove(0);
        return letters(c, document, &words, start, report, place);
    }
    let times = c.times();
    let end = times.last().copied().unwrap_or(0.0);
    let starts: Vec<f64> = runs.iter().map(|r| r.1).collect();
    let mut out = Vec::with_capacity(runs.len());
    for (n, (words, start)) in runs.iter().enumerate() {
        let until = starts.get(n + 1).copied().unwrap_or(end);
        let mut it = letters(c, document, words, *start, report, place);
        // Shown only from its own start until the next text's.
        let mut keys = Vec::new();
        let hold = |seconds: f64, v: f64| TrackKey {
            seconds,
            value: vec![v],
            hold: true,
            out: Vec::new(),
            inn: Vec::new(),
        };
        if n > 0 {
            keys.push(hold(starts.first().copied().unwrap_or(0.0), 0.0));
        }
        keys.push(hold(*start, 100.0));
        if n + 1 < runs.len() {
            keys.push(hold(until, 0.0));
        }
        it.push(identity(c.write(&Track { keys }, Class::Px, false, &[])));
        out.push(json!({"ty": "gr", "it": it}));
    }
    out.reverse();
    out
}

/// A group transform that only fades.
fn identity(opacity: Value) -> Value {
    json!({
        "ty": "tr",
        "a": {"a": 0, "k": [0, 0]},
        "p": {"a": 0, "k": [0, 0]},
        "s": {"a": 0, "k": [100, 100]},
        "r": {"a": 0, "k": 0},
        "o": opacity,
    })
}

/// `words` as one group per letter, read at layer time `lt` for anything
/// that isn't sampled.
fn letters(
    c: &Clock,
    document: &TextDocument,
    words: &str,
    lt: f64,
    report: &mut Report,
    place: &Place,
) -> Vec<Value> {
    let block = TextBlock::of(document, words);
    let spine = document
        .path
        .map(|id| lumit_core::mask::mask_path_at(&c.layer.masks, Some(id), false, lt))
        .filter(|p| !p.is_empty());
    let on_path = spine.is_some();
    if on_path {
        let mask_moves = c
            .layer
            .masks
            .iter()
            .any(|m| Some(m.id) == document.path && !m.path_keys.is_empty());
        if mask_moves || Clock::moves(&document.path_offset) {
            report.changed(
                place,
                "text_on_path_still",
                "Text on a path is drawn where it starts".into(),
                Vec::new(),
            );
        }
    }
    let glyphs = lumit_text::shape_items(
        &block,
        spine.as_ref(),
        c.value(&document.path_offset, lt) as f32,
    );

    let animated = !document.animators.is_empty() && !words.is_empty();
    if animated && on_path {
        report.left_out(
            place,
            "text_animators",
            "Text animators are left out on a path".into(),
            Vec::new(),
        );
    }
    let animated = animated && !on_path;
    let layout = lumit_text::layout(&block, animated);
    let margin = if animated {
        f64::from(lumit_text::animator_margin(block.size))
    } else {
        0.0
    };
    // Each letter's pen and advance on its line, by character index.
    let mut pens: Vec<Option<(f64, f64, f64)>> = vec![None; words.chars().count()];
    for line in &layout.lines {
        for (j, pair) in line.carets.windows(2).enumerate() {
            if let (Some(slot), [a, b]) = (pens.get_mut(line.start + j), pair) {
                *slot = Some((f64::from(*a), f64::from(b - a), f64::from(line.baseline)));
            }
        }
    }
    // Every letter's animation, sampled once for all of them.
    let samples: Vec<(f64, Vec<GlyphXform>)> = if animated {
        c.times()
            .into_iter()
            .take_while(|_| !c.host.cancelled())
            .map(|t| {
                (
                    t,
                    lumit_core::text::glyph_xforms(&document.animators, words, t),
                )
            })
            .collect()
    } else {
        Vec::new()
    };

    let chars: Vec<char> = words.chars().collect();
    let mut cursor = 0usize;
    let mut out = Vec::with_capacity(glyphs.len());
    for glyph in &glyphs {
        let ShapeEntry::Group(group) = glyph else {
            continue;
        };
        // The character this letter is, found in order by its name.
        while chars
            .get(cursor)
            .is_some_and(|ch| ch.to_string() != group.name)
        {
            cursor += 1;
        }
        let index = cursor;
        cursor += 1;
        let mut it = shapes::contents(c, &group.entries, report, place);
        let pen = pens.get(index).copied().flatten();
        match (animated, pen) {
            (true, Some((x, advance, baseline))) => {
                let pivot = (x + advance / 2.0, baseline);
                let series = |f: &dyn Fn(&GlyphXform) -> Vec<f64>| -> Vec<(f64, Vec<f64>)> {
                    samples
                        .iter()
                        .map(|(t, xs)| (*t, f(&xs.get(index).copied().unwrap_or_default())))
                        .collect()
                };
                let p = fit::linear(
                    &series(&|x| {
                        vec![
                            pivot.0 + f64::from(x.position[0]),
                            pivot.1 + f64::from(x.position[1]),
                        ]
                    }),
                    &[tolerance::PIXELS; 2],
                );
                let r = fit::linear(
                    &series(&|x| vec![f64::from(x.rotation)]),
                    &[tolerance::DEGREES],
                );
                let s = fit::linear(
                    &series(&|x| {
                        vec![f64::from(x.scale[0]) * 100.0, f64::from(x.scale[1]) * 100.0]
                    }),
                    &[tolerance::PERCENT; 2],
                );
                let o = fit::linear(
                    &series(&|x| vec![f64::from(x.opacity) * 100.0]),
                    &[tolerance::PERCENT],
                );
                let tinted = samples
                    .iter()
                    .any(|(_, xs)| xs.get(index).is_some_and(|x| x.fill != [0.0; 3]));
                if tinted {
                    let fill = document.fill.0.map(f64::from);
                    let colour = fit::linear(
                        &series(&|x| {
                            c.host
                                .display([0, 1, 2].map(|i| fill[i] + f64::from(x.fill[i])))
                                .to_vec()
                        }),
                        &[tolerance::CHANNEL; 3],
                    );
                    recolour(&mut it, &c.write(&colour, Class::Unit, true, &[]));
                }
                it.push(json!({
                    "ty": "tr",
                    "a": {"a": 0, "k": c.fmt.nums(&[pivot.0 - margin, pivot.1 - margin], Class::Px)},
                    "p": c.write(&p, Class::Px, true, &[]),
                    "s": c.write(&s, Class::Px, true, &[]),
                    "r": c.write(&r, Class::Deg, false, &[]),
                    "o": c.write(&o, Class::Px, false, &[]),
                }));
            }
            _ => it.push(identity(json!({"a": 0, "k": 100}))),
        }
        let mut g = Map::new();
        g.insert("ty".into(), Value::from("gr"));
        c.fmt.name(&mut g, &group.name);
        g.insert("it".into(), Value::Array(it));
        out.push(Value::Object(g));
    }
    out.reverse();
    out
}

/// Every fill in `items` given `colour`.
fn recolour(items: &mut [Value], colour: &Value) {
    for item in items {
        match item.get("ty").and_then(Value::as_str) {
            Some("fl") => {
                if let Some(m) = item.as_object_mut() {
                    m.insert("c".into(), colour.clone());
                }
            }
            Some("gr") => {
                if let Some(Value::Array(inner)) = item.get_mut("it") {
                    recolour(inner, colour);
                }
            }
            _ => {}
        }
    }
}

/// Where a Lottie text layer's origin (the first baseline, at the
/// alignment's edge) sits in the layer's own pixels, and the layout of its
/// first words.
fn laid(c: &Clock, document: &TextDocument) -> ((f64, f64), lumit_text::BlockLayout) {
    let runs = texts(c, document);
    let first = runs
        .first()
        .map_or(document.text.as_str(), |r| r.0.as_str());
    let animated = !document.animators.is_empty() && !first.is_empty();
    let layout = lumit_text::layout(&TextBlock::of(document, first), animated);
    let x = match document.paragraph.align {
        TextAlign::Left => layout.left,
        TextAlign::Right => layout.right,
        TextAlign::Centre => (layout.left + layout.right) / 2.0,
    };
    let baseline = layout.lines.first().map_or(0.0, |l| l.baseline);
    ((f64::from(x), f64::from(baseline)), layout)
}

/// Where a Lottie text layer's origin sits in the layer's own pixels.
#[must_use]
pub fn origin(c: &Clock, document: &TextDocument) -> (f64, f64) {
    laid(c, document).0
}

/// A Lottie text layer's `t`. The font joins `fonts`.
pub fn document(
    c: &Clock,
    document: &TextDocument,
    fonts: &mut Vec<Value>,
    report: &mut Report,
    place: &Place,
) -> Value {
    let style = &document.style;
    let family = if style.family.is_empty() {
        "Inter"
    } else {
        &style.family
    };
    let face = if style.face.is_empty() {
        "Regular"
    } else {
        &style.face
    };
    let font_name: String = format!("{family}-{face}")
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    // Every player draws it only with the font installed, so it is said
    // whichever players are ticked.
    report.changed(
        place,
        "text_needs_font",
        format!("Needs {family} {face} installed"),
        vec![arg("font", format!("{family} {face}"))],
    );
    let runs = texts(c, document);
    let (_, layout) = laid(c, document);
    if !fonts
        .iter()
        .any(|f| f.get("fName").and_then(Value::as_str) == Some(&font_name))
    {
        let size = document.size.max(1.0);
        fonts.push(json!({
            "fName": font_name,
            "fFamily": family,
            "fStyle": face,
            "ascent": c.fmt.num(f64::from(layout.ascent) / size * 100.0, Class::Px),
            "origin": 0,
        }));
    }
    let j = match document.paragraph.align {
        TextAlign::Left => 0,
        TextAlign::Right => 1,
        TextAlign::Centre => 2,
    };
    let fill = document.fill.0.map(f64::from);
    let fc = c.host.display([fill[0], fill[1], fill[2]]);
    let keys: Vec<Value> = runs
        .iter()
        .map(|(words, lt)| {
            let mut s = Map::new();
            s.insert("t".into(), Value::from(words.replace('\n', "\r")));
            s.insert("s".into(), c.fmt.num(document.size, Class::Px));
            s.insert("f".into(), Value::from(font_name.clone()));
            s.insert("j".into(), Value::from(j));
            s.insert("tr".into(), c.fmt.num(style.tracking, Class::Px));
            s.insert(
                "lh".into(),
                c.fmt
                    .num(style.leading.unwrap_or(document.size * 1.2), Class::Px),
            );
            s.insert("ls".into(), c.fmt.num(style.baseline_shift, Class::Px));
            s.insert("fc".into(), c.fmt.nums(&fc, Class::Unit));
            if style.stroke_on && style.stroke_width > 0.0 {
                let sc = style.stroke.0.map(f64::from);
                s.insert(
                    "sc".into(),
                    c.fmt
                        .nums(&c.host.display([sc[0], sc[1], sc[2]]), Class::Unit),
                );
                s.insert("sw".into(), c.fmt.num(style.stroke_width, Class::Px));
                s.insert("of".into(), Value::from(style.stroke_over));
            }
            json!({"t": c.fmt.num(c.frame(*lt), Class::Time), "s": Value::Object(s)})
        })
        .collect();
    if document.path.is_some() {
        report.players(
            place,
            Feature::TextOnPath,
            "text_on_path",
            "Text on a path".into(),
            Vec::new(),
        );
    }
    let animators: Vec<Value> = document
        .animators
        .iter()
        .map(|a| {
            let sel = &a.selector;
            let mut m = Map::new();
            c.fmt.name(&mut m, &a.name);
            m.insert(
                "s".into(),
                json!({
                    "t": 0,
                    "xe": {"a": 0, "k": 0},
                    "ne": {"a": 0, "k": 0},
                    "a": {"a": 0, "k": 100},
                    "b": if sel.basis == SelectorBasis::Words { 3 } else { 1 },
                    "rn": 0,
                    "sh": if sel.shape == SelectorShape::Ramp { 2 } else { 1 },
                    "r": 1,
                    "s": c.prop(&[&sel.start], Class::Px, tolerance::PERCENT),
                    "e": c.prop(&[&sel.end], Class::Px, tolerance::PERCENT),
                    "o": c.prop(&[&sel.offset], Class::Px, tolerance::PERCENT),
                    "sm": {"a": 0, "k": 100},
                }),
            );
            m.insert(
                "a".into(),
                json!({
                    "p": c.prop(&[&a.position_x, &a.position_y], Class::Px, tolerance::PIXELS),
                    "r": c.prop(&[&a.rotation], Class::Deg, tolerance::DEGREES),
                    "s": c.prop(&[&a.scale_x, &a.scale_y], Class::Px, tolerance::PERCENT),
                    "o": c.prop(&[&a.opacity], Class::Px, tolerance::PERCENT),
                }),
            );
            let tints = [&a.fill_r, &a.fill_g, &a.fill_b]
                .iter()
                .any(|p| !matches!(p.animation, Animation::Static(v) if v == 0.0));
            if tints {
                report.left_out(
                    place,
                    "text_fill_animator",
                    "Animator fill is left out".into(),
                    vec![arg("label", a.name.clone())],
                );
            }
            Value::Object(m)
        })
        .collect();
    if !animators.is_empty() {
        report.players(
            place,
            Feature::TextAnimators,
            "text_animators",
            "Text animators".into(),
            Vec::new(),
        );
    }
    let t = json!({
        "d": {"k": keys},
        "a": animators,
        "m": {"g": 1, "a": {"a": 0, "k": [0, 0]}},
        "p": {},
    });
    t
}

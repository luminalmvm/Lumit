use std::ops::Range;

use super::*;
use crate::{
    captions::cues::CueWord,
    model::{Composition, Document, LayerKind, LinearColour, ProjectItem, TextDocument},
    sequence::Clip,
    time::{CompTime, Duration, FrameRate, Rational},
};

fn s(ms: i64) -> Rational {
    Rational::new(ms, 1000).unwrap()
}

fn word(text: &str, from: i64, to: i64) -> CueWord {
    CueWord {
        text: text.into(),
        start: s(from),
        end: s(to),
        speaker: None,
        emphasis: false,
    }
}

fn layer(name: &str, kind: LayerKind) -> Layer {
    crate::edits::base_layer(
        name.into(),
        kind,
        s(20_000),
        crate::model::TransformGroup::default(),
    )
}

fn key(sec: i64, value: f64) -> crate::anim::Keyframe {
    crate::anim::Keyframe {
        time: Rational::new(sec, 1).unwrap(),
        value,
        interp_in: crate::anim::SideInterp::Linear,
        interp_out: crate::anim::SideInterp::Linear,
    }
}

fn comp(name: &str, layers: Vec<Layer>) -> Composition {
    Composition {
        graph: None,
        master_volume_db: 0.0,
        master_keys: None,
        master_automation: Default::default(),
        loudness_target: None,
        lighting: Default::default(),
        blending: Default::default(),
        sound_mix: false,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
        name: name.into(),
        width: 1920,
        height: 1080,
        frame_rate: FrameRate::new(30, 1).unwrap(),
        duration: Duration(s(20_000)),
        background: LinearColour([0.0, 0.0, 0.0, 1.0]),
        work_area: None,
        layers,
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    }
}

fn cue(item: Option<Uuid>, words: Vec<CueWord>) -> Cue {
    Cue {
        start: words[0].start,
        end: words[words.len() - 1].end,
        words,
        breaks: Vec::new(),
        label: None,
        dashes: false,
        item,
        offset: None,
        extra: serde_json::Map::new(),
    }
}

fn captions(source: Option<Uuid>, cues: Vec<Cue>) -> Captions {
    Captions {
        source,
        cues,
        language: "en".into(),
        limits: Default::default(),
        speakers: Vec::new(),
        extra: serde_json::Map::new(),
    }
}

fn caption_layer(captions: Captions) -> Layer {
    layer(
        "Captions",
        LayerKind::Text {
            document: TextDocument {
                text: String::new(),
                expression: None,
                keys: None,
                size: 60.0,
                fill: LinearColour([1.0, 1.0, 1.0, 1.0]),
                path: None,
                path_offset: crate::anim::Property::zero(),
                animators: Vec::new(),
                style: Default::default(),
                paragraph: Default::default(),
                captions: Some(Arc::new(captions)),
                extra: serde_json::Map::new(),
            },
        },
    )
}

fn holding(comps: Vec<Composition>) -> Arc<Document> {
    let mut doc = Document::new();
    doc.items
        .extend(comps.into_iter().map(ProjectItem::Composition));
    Arc::new(doc)
}

fn context(doc: &Arc<Document>, comp: Uuid, layer: Uuid, t: f64) -> ExpressionContext {
    ExpressionContext {
        document: Arc::clone(doc),
        comp: Some(comp),
        layer: Some(layer),
        comp_time: t,
        current_depth: 0,
        inputs: None,
    }
}

fn look(doc: &Arc<Document>, comp: Uuid, text: Uuid, t: f64) -> (Layer, Arc<Captions>, Shown) {
    let layer = doc
        .comp(comp)
        .unwrap()
        .layers
        .iter()
        .find(|l| l.id == text)
        .unwrap()
        .clone();
    let LayerKind::Text { document } = &layer.kind else {
        panic!()
    };
    let captions = document.captions.clone().unwrap();
    let seen = at(&captions, &context(doc, comp, text, t));
    (layer, captions, seen)
}

fn near(got: f64, want: f64) -> bool {
    (got - want).abs() < 1e-3
}

fn edit(item: Uuid) -> (Arc<Document>, Uuid, Uuid) {
    let picture = layer(
        "Talk",
        LayerKind::Sequence {
            clips: vec![
                Clip::new(
                    crate::sequence::ClipSource::Footage(item),
                    s(0),
                    s(5200),
                    s(0),
                    s(5200),
                ),
                Clip::new(
                    crate::sequence::ClipSource::Footage(item),
                    s(5600),
                    s(10_000),
                    s(5200),
                    s(4400),
                ),
            ],
        },
    );
    let cue = Cue {
        start: s(4600),
        end: s(6600),
        words: vec![
            word("So", 4600, 4900),
            word("um", 5300, 5500),
            word("yes", 5600, 5900),
            word("please.", 6000, 6400),
        ],
        breaks: Vec::new(),
        label: None,
        dashes: false,
        item: Some(item),
        offset: None,
        extra: serde_json::Map::new(),
    };
    let captions = Captions {
        source: Some(picture.id),
        cues: vec![cue],
        language: "en".into(),
        limits: Default::default(),
        speakers: Vec::new(),
        extra: serde_json::Map::new(),
    };
    let text = layer(
        "Captions",
        LayerKind::Text {
            document: TextDocument {
                text: String::new(),
                expression: None,
                keys: None,
                size: 60.0,
                fill: LinearColour([1.0, 1.0, 1.0, 1.0]),
                path: None,
                path_offset: crate::anim::Property::zero(),
                animators: Vec::new(),
                style: Default::default(),
                paragraph: Default::default(),
                captions: Some(Arc::new(captions)),
                extra: serde_json::Map::new(),
            },
        },
    );
    let (picture_id, text_id) = (picture.id, text.id);
    let comp = Composition {
        graph: None,
        master_volume_db: 0.0,
        master_keys: None,
        master_automation: Default::default(),
        loudness_target: None,
        lighting: Default::default(),
        blending: Default::default(),
        sound_mix: false,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
        name: "Edit".into(),
        width: 1920,
        height: 1080,
        frame_rate: FrameRate::new(30, 1).unwrap(),
        duration: Duration(s(20_000)),
        background: LinearColour([0.0, 0.0, 0.0, 1.0]),
        work_area: None,
        layers: vec![text, picture],
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    };
    let mut doc = Document::new();
    doc.items.push(ProjectItem::Composition(comp));
    (Arc::new(doc), picture_id, text_id)
}

fn shown(doc: &Arc<Document>, text: Uuid, t: f64) -> Shown {
    let comp = doc
        .items
        .iter()
        .find_map(|i| match i {
            ProjectItem::Composition(c) => Some(c),
            _ => None,
        })
        .unwrap();
    let layer = comp.layers.iter().find(|l| l.id == text).unwrap();
    let LayerKind::Text { document } = &layer.kind else {
        panic!()
    };
    let context = ExpressionContext {
        document: Arc::clone(doc),
        comp: Some(comp.id),
        layer: Some(text),
        comp_time: t,
        current_depth: 0,
        inputs: None,
    };
    at(document.captions.as_ref().unwrap(), &context)
}

#[test]
fn linked_captions_follow_the_cut() {
    let item = Uuid::now_v7();
    let (doc, _, text) = edit(item);
    for t in [4.7, 5.3] {
        let seen = shown(&doc, text, t);
        assert_eq!(seen.text, "So yes please.", "at {t}");
        let starts: Vec<f64> = seen.words.iter().map(|w| w.start).collect();
        for (got, want) in starts.iter().zip([4.6, 5.2, 5.6]) {
            assert!((got - want).abs() < 1e-6, "at {t}: {starts:?}");
        }
    }
    assert!(
        shown(&doc, text, 3.0).text.is_empty(),
        "nothing before the cue"
    );

    let mut moved = (*doc).clone();
    let ProjectItem::Composition(comp) = &mut moved.items[0] else {
        panic!()
    };
    for layer in &mut comp.layers {
        match &mut layer.kind {
            LayerKind::Sequence { clips } => {
                let first = clips[0].clone();
                clips[0] = Clip::new(first.source, s(0), s(1000), s(0), s(1000));
                let mut rest = Clip::new(first.source, s(2000), s(5200), s(1000), s(3200));
                rest.id = first.id;
                clips.insert(1, rest);
                clips[2].place_start = s(4200);
            }
            LayerKind::Text { .. } => {
                layer.in_point = CompTime(s(2000));
                layer.start_offset = CompTime(s(2000));
            }
            _ => {}
        }
    }
    let moved = Arc::new(moved);
    let before = shown(&doc, text, 4.7);
    let after = shown(&moved, text, 3.7);
    assert_eq!(after.text, before.text);
    for (a, b) in after.words.iter().zip(&before.words) {
        assert!(
            ((a.start + 2.0) - (b.start - 1.0)).abs() < 1e-6,
            "{a:?} against {b:?}"
        );
    }

    let mut gone = (*doc).clone();
    let ProjectItem::Composition(comp) = &mut gone.items[0] else {
        panic!()
    };
    comp.layers
        .retain(|l| matches!(l.kind, LayerKind::Text { .. }));
    assert!(shown(&Arc::new(gone), text, 4.7).text.is_empty());
}

#[test]
fn captions_on_their_own_clock_show_and_export_in_step() {
    let mut first = cue(
        None,
        vec![word("Hello", 500, 1000), word("there.", 1000, 1500)],
    );
    first.label = Some("Ann:".into());
    first.end = s(2000);
    let mut second = cue(
        None,
        vec![word("Yes.", 2000, 2500), word("No.", 2600, 3500)],
    );
    second.dashes = true;
    second.breaks = vec![1];
    second.offset = Some([0.0, -500.0]);
    let mut text = caption_layer(captions(None, vec![first, second]));
    text.in_point = CompTime(s(1000));
    text.start_offset = CompTime(s(1000));
    let text_id = text.id;
    let subtitles = comp("Subtitles", vec![text]);
    let comp_id = subtitles.id;
    let doc = holding(vec![subtitles]);

    let (layer, held, seen) = look(&doc, comp_id, text_id, 1.6);
    assert_eq!(seen.text, "Ann: Hello there.");
    let chars: Vec<Range<usize>> = seen.words.iter().map(|w| w.chars.clone()).collect();
    assert_eq!(chars, vec![5..10, 11..17]);
    assert_eq!((seen.words[0].start, seen.words[1].start), (0.5, 1.0));
    let later = look(&doc, comp_id, text_id, 3.5).2;
    assert_eq!(later.text, "- Yes.\n- No.");
    assert!(
        look(&doc, comp_id, text_id, 0.9).2.text.is_empty(),
        "nothing before the first"
    );

    let here = context(&doc, comp_id, text_id, 0.0);
    let then = |t: f64| context(&doc, comp_id, text_id, t);
    assert_eq!(moved(&layer, &then(3.5)), (0.0, -500.0));
    assert_eq!(moved(&layer, &then(1.6)), (0.0, 0.0));

    let LayerKind::Text { document } = &layer.kind else {
        panic!()
    };
    let (said, words, _) = spoken(document, Arc::new(then(1.6)));
    assert_eq!((said.as_ref(), words.len()), ("Ann: Hello there.", 2));
    let plain = TextDocument {
        text: "Plain".into(),
        captions: None,
        ..document.clone()
    };
    let (said, words, _) = spoken(&plain, Arc::new(here.clone()));
    assert_eq!((said.as_ref(), words.len()), ("Plain", 0));

    let times: Vec<f64> = (0..20).map(|f| f64::from(f) * 0.25).collect();
    let written = spans(&held, &here, &times, 5.0, &|| false);
    let placed: Vec<(f64, f64)> = written
        .iter()
        .map(|c| (c.start.to_f64(), c.end.to_f64()))
        .collect();
    assert_eq!(placed, vec![(1.5, 3.0), (3.0, 4.5)]);
    assert_eq!(written[0].words[0].start.to_f64(), 1.5);
    assert_eq!(written[0].label.as_deref(), Some("Ann:"));
    assert_eq!(written[1].breaks, vec![1]);

    let subtitles = doc.comp(comp_id).unwrap();
    assert_eq!(
        first_shown(&doc, &held, subtitles, &layer),
        vec![Some((1.5, 3.0)), Some((3.0, 4.5))]
    );
}

#[test]
fn linked_captions_are_heard_through_a_retime_and_a_precomp() {
    let item = Uuid::now_v7();
    let words = vec![word("fast", 4000, 5000), word("talk", 5000, 6000)];

    let mut fast = layer("Fast", LayerKind::Footage { item });
    let mut retime = crate::anim::Property::fixed(0.0);
    retime.animation = crate::anim::Animation::Keyframed(vec![key(0, 0.0), key(10, 20.0)]);
    fast.retime = Some(retime);
    let text = caption_layer(captions(
        Some(fast.id),
        vec![cue(Some(item), words.clone())],
    ));
    let text_id = text.id;
    let sped = comp("Fast", vec![text, fast]);
    let comp_id = sped.id;
    let doc = holding(vec![sped]);
    let (captioned, held, seen) = look(&doc, comp_id, text_id, 2.5);
    assert_eq!(seen.text, "fast talk");
    let times: Vec<(f64, f64)> = seen.words.iter().map(|w| (w.start, w.end)).collect();
    assert!(
        near(times[0].0, 2.0) && near(times[1].0, 2.5) && near(times[1].1, 3.0),
        "{times:?}"
    );
    let placed = first_shown(&doc, &held, doc.comp(comp_id).unwrap(), &captioned);
    assert!(
        placed[0].is_some_and(|(a, b)| near(a, 2.0) && near(b, 3.0)),
        "{placed:?}"
    );

    let mut slow = layer("Slow", LayerKind::Footage { item });
    let mut retime = crate::anim::Property::fixed(0.0);
    retime.animation = crate::anim::Animation::Keyframed(vec![key(0, 0.0), key(20, 5.0)]);
    slow.retime = Some(retime);
    let long = vec![word("slow", 0, 2000), word("talk", 2000, 4000)];
    let text = caption_layer(captions(Some(slow.id), vec![cue(Some(item), long)]));
    let text_id = text.id;
    let slowed = comp("Slow", vec![text, slow]);
    let slow_id = slowed.id;
    let doc = holding(vec![slowed]);
    assert_eq!(look(&doc, slow_id, text_id, 15.5).2.text, "slow talk");

    let broll = Uuid::now_v7();
    let nested = comp(
        "Nested",
        vec![layer_named("B-roll", broll), layer_named("Talk", item)],
    );
    let mut precomp = layer("Nested", LayerKind::Precomp { comp: nested.id });
    precomp.in_point = CompTime(s(1000));
    precomp.start_offset = CompTime(s(1000));
    let text = caption_layer(captions(Some(precomp.id), vec![cue(Some(item), words)]));
    let text_id = text.id;
    let outer = comp("Outer", vec![text, precomp]);
    let outer_id = outer.id;
    let doc = holding(vec![nested, outer]);
    let (captioned, held, seen) = look(&doc, outer_id, text_id, 5.5);
    assert_eq!(seen.text, "fast talk");
    assert!(near(seen.words[0].start, 5.0), "{:?}", seen.words);
    let outer = doc.comp(outer_id).unwrap();
    let anything = heard_at(&doc, &outer.layers[1], 5.5, &|_, _| true);
    assert_eq!(anything.map(|h| h.item), Some(broll));
    let placed = first_shown(&doc, &held, outer, &captioned);
    assert!(
        placed[0].is_some_and(|(a, b)| near(a, 5.0) && near(b, 7.0)),
        "{placed:?}"
    );
}

fn layer_named(name: &str, item: Uuid) -> Layer {
    layer(name, LayerKind::Footage { item })
}

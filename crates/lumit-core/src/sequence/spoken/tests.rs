#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use super::*;
use crate::anim::{Animation, Keyframe, Property, SideInterp};
use crate::model::{AudioStreams, FootageItem, MediaRef, ProjectItem};
use crate::transcript::{Transcript, Word};

fn ms(n: i64) -> Rational {
    Rational::new(n, 1000).unwrap()
}

fn footage(doc: &mut Document) -> Uuid {
    let id = Uuid::now_v7();
    doc.items.push(ProjectItem::Footage(FootageItem {
        id,
        name: "Talk.mp4".into(),
        media: MediaRef {
            relative_path: "Talk.mp4".into(),
            absolute_path: String::new(),
            fingerprint: None,
            extra: serde_json::Map::new(),
        },
        colour_space: None,
        sequence: None,
        source_layer: None,
        interpretation: crate::interpretation::Interpretation::default(),
        extra: serde_json::Map::new(),
    }));
    id
}

/// Land a transcript on `item`: each word as text, start and end in ms, and
/// how sure the model was.
fn transcribe(doc: &mut Document, item: Uuid, language: &str, words: &[(&str, i64, i64, f32)]) {
    let words = words
        .iter()
        .map(|(text, a, b, c)| Word::new(*text, SourceTime(ms(*a)), SourceTime(ms(*b)), *c))
        .collect();
    let transcript = Transcript {
        language: language.into(),
        stream: 0,
        words,
        speakers: Vec::new(),
        model: String::new(),
        extra: serde_json::Map::new(),
    };
    doc.transcripts.insert(item, Arc::new(transcript));
}

fn comp(doc: &mut Document, fps: u32) -> Uuid {
    let settings = crate::edits::CompSettings {
        frame_rate: FrameRate::new(fps, 1).unwrap(),
        ..Default::default()
    };
    let (id, ops) = crate::edits::new_comp_ops(doc, settings, None);
    for op in ops {
        crate::ops::apply(doc, &op).unwrap();
    }
    id
}

/// A layer on top of `comp` holding `kind`, from 0 to 30 s.
fn layer(doc: &mut Document, comp: Uuid, kind: LayerKind, audio_only: bool) -> Uuid {
    let mut layer = crate::edits::base_layer(
        "Row".into(),
        kind,
        Rational::new(30, 1).unwrap(),
        Default::default(),
    );
    layer.audio_only = audio_only;
    let id = layer.id;
    doc.comp_mut(comp).unwrap().layers.insert(0, layer);
    id
}

fn row(doc: &mut Document, comp: Uuid, clips: Vec<Clip>) -> Uuid {
    layer(doc, comp, LayerKind::Sequence { clips }, true)
}

fn clip(item: Uuid, source_in: i64, source_out: i64, place: i64) -> Clip {
    Clip::new(
        ClipSource::Footage(item),
        ms(source_in),
        ms(source_out),
        ms(place),
        ms(source_out - source_in),
    )
}

fn layer_mut(doc: &mut Document, comp: Uuid, id: Uuid) -> &mut Layer {
    let comp = doc.comp_mut(comp).unwrap();
    comp.layers.iter_mut().find(|l| l.id == id).unwrap()
}

/// Each heard word as (its place in the transcript, start ms, end ms).
fn said(doc: &Document, comp: Uuid) -> Vec<(usize, f64, f64)> {
    heard(doc, doc.comp(comp).unwrap())
        .iter()
        .map(|h| (h.index, h.start.to_f64() * 1000.0, h.end.to_f64() * 1000.0))
        .collect()
}

const TALK: &[(&str, i64, i64, f32)] = &[
    ("Hello", 0, 400, 0.9),
    ("there", 500, 900, 0.9),
    ("friend", 1500, 1900, 0.9),
    ("again", 2000, 2400, 0.9),
];

/// **Which words a cut says.** A word is heard where its clip puts it, only
/// while its middle is inside the trim, only from the stream that was
/// transcribed, and not through a silent clip, a muted layer or past a solo.
/// The panel deletes media by these times, so a wrong one cuts the wrong
/// sound.
#[test]
fn a_cut_says_the_words_its_heard_clips_play() {
    let mut doc = Document::new();
    let item = footage(&mut doc);
    transcribe(&mut doc, item, "en", TALK);
    let c = comp(&mut doc, 25);
    // The sound plays 0.5 s to 2 s of the file, from 10 s on a layer whose
    // zero is 1 s into the comp.
    let sound = row(&mut doc, c, vec![clip(item, 500, 2000, 10_000)]);
    layer_mut(&mut doc, c, sound).start_offset = CompTime(ms(1000));
    // Its picture is silent, so nothing is heard twice.
    let mut picture = clip(item, 500, 2000, 11_000);
    picture.audible = false;
    layer(
        &mut doc,
        c,
        LayerKind::Sequence {
            clips: vec![picture],
        },
        false,
    );
    assert_eq!(
        said(&doc, c),
        [(1, 11_000.0, 11_400.0), (2, 12_000.0, 12_400.0)]
    );

    // The transcript heard the second stream, which this clip does not play.
    let mut on_two = (*doc.transcripts[&item]).clone();
    on_two.stream = 1;
    doc.transcripts.insert(item, Arc::new(on_two));
    assert!(said(&doc, c).is_empty());
    if let LayerKind::Sequence { clips } = &mut layer_mut(&mut doc, c, sound).kind {
        clips[0].audio_streams = AudioStreams::only(1);
    }
    assert_eq!(said(&doc, c).len(), 2);

    // A soloed sound elsewhere leaves this layer unheard, and so does muting it.
    let solo = row(&mut doc, c, Vec::new());
    layer_mut(&mut doc, c, solo).switches.solo = true;
    assert!(said(&doc, c).is_empty());
    layer_mut(&mut doc, c, solo).switches.solo = false;
    layer_mut(&mut doc, c, sound).switches.audible = false;
    assert!(said(&doc, c).is_empty());
}

/// A key on a clip's Retime map.
fn key(time: i64, value: f64) -> Keyframe {
    Keyframe {
        time: ms(time),
        value,
        interp_in: SideInterp::Linear,
        interp_out: SideInterp::Linear,
    }
}

/// **Retimed clips.** At one speed a word is said in proportion. Through a
/// ramp it is found where the source reaches it going forwards, and a
/// stretch played backwards or frozen says no words.
#[test]
fn retimed_clips_say_their_words_where_the_map_puts_them() {
    let mut doc = Document::new();
    let item = footage(&mut doc);
    transcribe(&mut doc, item, "en", TALK);
    let c = comp(&mut doc, 25);

    let double = clip(item, 0, 1000, 0)
        .with_ramp(Rational::new(2, 1).unwrap(), Rational::new(2, 1).unwrap());
    let r = row(&mut doc, c, vec![double]);
    let words = said(&doc, c);
    assert_eq!(words.iter().map(|w| w.0).collect::<Vec<_>>(), [0, 1, 2]);
    assert!((words[1].1 - 250.0).abs() < 0.01 && (words[1].2 - 450.0).abs() < 0.01);

    // Forwards to 1 s of source in the first half second, then back to 0.5 s.
    let mut ramp = clip(item, 0, 1000, 0);
    ramp.retime = Some(Property {
        animation: Animation::Keyframed(vec![key(0, 0.0), key(500, 1.0), key(1000, 0.5)]),
        extra: serde_json::Map::new(),
    });
    if let LayerKind::Sequence { clips } = &mut layer_mut(&mut doc, c, r).kind {
        *clips = vec![ramp.clone()];
    }
    let words = said(&doc, c);
    assert_eq!(
        words.iter().map(|w| w.0).collect::<Vec<_>>(),
        [0, 1],
        "each once, going forwards"
    );
    assert!((words[1].1 - 250.0).abs() < 1.0, "{words:?}");

    // Played backwards: nothing said.
    ramp.retime = Some(Property {
        animation: Animation::Keyframed(vec![key(0, 1.0), key(1000, 0.0)]),
        extra: serde_json::Map::new(),
    });
    if let LayerKind::Sequence { clips } = &mut layer_mut(&mut doc, c, r).kind {
        *clips = vec![ramp];
    }
    assert!(said(&doc, c).is_empty());
}

/// **Nesting, Footage layers and mic bleed.** A clip that plays a
/// composition says what that composition says, through its own trim. A
/// Footage layer says its file's words from its own zero, held to its span.
/// The same word picked up by two microphones at once is said once, by the
/// one that heard it best.
#[test]
fn nested_comps_footage_layers_and_bleed() {
    let mut doc = Document::new();
    let a = footage(&mut doc);
    transcribe(&mut doc, a, "en", TALK);
    let inner = comp(&mut doc, 25);
    layer(&mut doc, inner, LayerKind::Footage { item: a }, true);
    let outer = comp(&mut doc, 25);
    let nested = Clip::new(
        ClipSource::Comp(inner),
        ms(1000),
        ms(3000),
        Rational::ZERO,
        ms(2000),
    );
    let solid = Clip::new(
        ClipSource::Solid(Uuid::now_v7()),
        Rational::ZERO,
        ms(500),
        ms(2000),
        ms(500),
    );
    row(&mut doc, outer, vec![nested, solid]);
    assert_eq!(said(&doc, outer), [(2, 500.0, 900.0), (3, 1000.0, 1400.0)]);

    // A second microphone on a Footage layer that starts 1 s in and stops at
    // 2.2 s: it hears "friend" faintly, and a word of its own.
    let b = footage(&mut doc);
    transcribe(
        &mut doc,
        b,
        "en",
        &[("Friend.", 1510, 1880, 0.4), ("yes", 1950, 2300, 0.8)],
    );
    let mic = layer(&mut doc, inner, LayerKind::Footage { item: b }, true);
    let mic = layer_mut(&mut doc, inner, mic);
    mic.in_point = CompTime(ms(1000));
    mic.out_point = CompTime(ms(2200));
    let heard = heard(&doc, doc.comp(outer).unwrap());
    let items: Vec<(Uuid, usize)> = heard.iter().map(|h| (h.item, h.index)).collect();
    assert_eq!(items, [(a, 2), (b, 1), (a, 3)], "the faint copy went");
    assert_eq!(heard[1].end, ms(1200), "held to the layer's out point");

    // A multicam clip says its own angle's words and no other's, and a clip
    // of footage takes no notice of an angle left on it.
    let angles = comp(&mut doc, 25);
    let wide = row(&mut doc, angles, vec![clip(a, 0, 4000, 0)]);
    let close = row(&mut doc, angles, vec![clip(b, 0, 4000, 0)]);
    let cut = comp(&mut doc, 25);
    let mut through = Clip::new(
        ClipSource::Comp(angles),
        Rational::ZERO,
        ms(4000),
        Rational::ZERO,
        ms(4000),
    );
    through.angle = Some(close);
    let mut take = clip(a, 0, 4000, 4000);
    take.angle = Some(wide);
    row(&mut doc, cut, vec![through, take]);
    let through = super::heard(&doc, doc.comp(cut).unwrap());
    let (angle, after): (Vec<&Heard>, Vec<&Heard>) =
        through.iter().partition(|h| h.start < ms(4000));
    assert_eq!(
        angle.iter().map(|h| (h.item, h.index)).collect::<Vec<_>>(),
        [(b, 0), (b, 1)],
        "the close angle alone"
    );
    assert!(after.iter().all(|h| h.item == a) && after.len() == TALK.len());
}

const PLAN: &[(&str, i64, i64, f32)] = &[
    ("So,", 0, 300, 0.9),
    ("um", 400, 600, 0.0),
    ("the", 700, 900, 0.9),
    ("plan", 2000, 2400, 0.9),
    ("is", 2500, 2700, 0.9),
    ("is", 2800, 3000, 0.9),
    ("simple.", 3100, 3600, 0.9),
];

/// **The layout edits take.** Tokens tile the cut on frame edges with no
/// hole, a long silence inside a clip is a pause that leaves each word its
/// lead, fillers are marked, and shortening a pause takes the middle of the
/// silence. A hole or an overlap here would delete media nobody selected.
#[test]
fn tokens_tile_the_cut_and_mark_pauses_and_fillers() {
    let mut doc = Document::new();
    let item = footage(&mut doc);
    transcribe(&mut doc, item, "en", PLAN);
    let c = comp(&mut doc, 25);
    row(&mut doc, c, vec![clip(item, 0, 4000, 0)]);
    let tokens = tokens(&doc, doc.comp(c).unwrap(), &Look::default());
    for pair in tokens.windows(2) {
        assert_eq!(pair[0].end, pair[1].start, "no hole and no overlap");
    }
    assert_eq!(
        (tokens[0].start, tokens.last().unwrap().end),
        (Rational::ZERO, ms(4000))
    );
    let kinds: Vec<TokenKind> = tokens.iter().map(|t| t.kind).collect();
    use TokenKind::{Pause, Word};
    assert_eq!(kinds, [Word, Word, Word, Pause, Word, Word, Word, Word]);
    // A lead of 150 ms either side, on the 40 ms grid.
    assert_eq!((tokens[3].start, tokens[3].end), (ms(1040), ms(1840)));
    let fillers: Vec<Option<Filler>> = tokens.iter().map(|t| t.filler).collect();
    assert_eq!(
        fillers,
        [
            Some(Filler::Phrase),
            Some(Filler::Hesitation),
            None,
            None,
            None,
            Some(Filler::Repeat),
            None,
            None
        ]
    );

    // 100 ms kept either side of the silence, 900 ms to 2 s, and the cut
    // inside it on whole frames.
    let cut = shorten_pauses(&tokens, ms(500), ms(200), FrameRate::new(25, 1).unwrap());
    assert_eq!(cut, [(ms(1000), ms(1880))]);
    assert_eq!(
        merged(vec![(ms(0), ms(40)), (ms(40), ms(80)), (ms(200), ms(120))]),
        [(ms(0), ms(80))]
    );

    // On its own clock every silence is a pause, and nothing names a layer.
    let own = source_tokens(&doc, item, FrameRate::new(25, 1).unwrap(), &Look::default());
    assert_eq!(own.iter().filter(|t| t.kind == Pause).count(), 1);
    assert!(own.iter().all(|t| t.row.is_none()));
}

/// **What a cut took out.** After an extract the words that were in the
/// middle come back as removed tokens at the edit point, the cut there falls
/// on the edit point even when the word before ends right on it, and the
/// word after it says an edit comes before it.
#[test]
fn an_extract_leaves_its_words_at_the_edit_point() {
    let mut doc = Document::new();
    let item = footage(&mut doc);
    transcribe(&mut doc, item, "en", PLAN);
    let c = comp(&mut doc, 25);
    // The extract trims "um" to end on the edit point.
    let (a, b) = (clip(item, 0, 560, 0), clip(item, 1920, 4000, 560));
    let first = a.id;
    row(&mut doc, c, vec![a, b]);
    let look = Look {
        show_removed: true,
        ..Look::default()
    };
    let tokens = tokens(&doc, doc.comp(c).unwrap(), &look);
    let gone: Vec<&Token> = tokens
        .iter()
        .filter(|t| t.kind == TokenKind::Removed)
        .collect();
    assert_eq!(gone.len(), 1);
    assert_eq!(gone[0].word, Some(WordAt { item, index: 2 }));
    assert_eq!(
        (gone[0].start, gone[0].end, gone[0].clip),
        (ms(560), ms(560), Some(first))
    );
    let plan = tokens
        .iter()
        .position(|t| t.word.is_some_and(|w| w.index == 3))
        .unwrap();
    assert_eq!(tokens[plan].start, ms(560));
    assert!(tokens[plan].edit_before);
}

/// Fillers in every language with a list: each one's first hesitation is
/// one, and a phrase set off by a pause is one.
#[test]
fn every_language_has_its_fillers() {
    for language in [
        "en", "de", "fr", "es", "it", "pt", "nl", "sv", "da", "nb", "pl", "ru", "tr", "ja", "zh",
        "ko",
    ] {
        let mut doc = Document::new();
        let item = footage(&mut doc);
        let base = fillers::base(language);
        let hesitation = fillers::hesitations(&base)[0];
        let phrase = fillers::phrases(&base)[0].split(' ').next().unwrap();
        transcribe(
            &mut doc,
            item,
            language,
            &[(hesitation, 0, 200, 0.9), (phrase, 1000, 1200, 0.9)],
        );
        let c = comp(&mut doc, 25);
        row(&mut doc, c, vec![clip(item, 0, 2000, 0)]);
        let marked: Vec<Option<Filler>> = tokens(&doc, doc.comp(c).unwrap(), &Look::default())
            .iter()
            .filter_map(|t| t.word.map(|_| t.filler))
            .collect();
        assert_eq!(marked[0], Some(Filler::Hesitation), "{language}");
        if !fillers::phrases(&base)[0].contains(' ') {
            assert_eq!(marked[1], Some(Filler::Phrase), "{language}");
        }
    }
}

/// **A script finds its takes.** Names and directions are not lines, a line
/// is found where enough of it is said, the better take comes first, and a
/// line nobody said finds nothing.
#[test]
fn a_script_finds_each_line_and_its_takes() {
    let script =
        "JOHN\nWe should go home now.\n(beat)\nIt is late. Very late!\n\nMARY (V.O.)\nNever said.";
    let lines = script::lines(script);
    assert_eq!(
        lines,
        [
            "We should go home now.",
            "It is late.",
            "Very late!",
            "Never said."
        ]
    );

    let mut doc = Document::new();
    let (one, two) = (footage(&mut doc), footage(&mut doc));
    let take = |words: &str, from: i64| -> Vec<(String, i64, i64)> {
        words
            .split(' ')
            .enumerate()
            .map(|(i, w)| {
                (
                    w.to_owned(),
                    from + i as i64 * 300,
                    from + i as i64 * 300 + 250,
                )
            })
            .collect()
    };
    let mut first = take("okay we should go home now", 0);
    first.extend(take("it is so late", 3000));
    let second = take("we should go uh home right now", 0);
    for (item, words) in [(one, first), (two, second)] {
        let words: Vec<(&str, i64, i64, f32)> = words
            .iter()
            .map(|(w, a, b)| (w.as_str(), *a, *b, 0.9))
            .collect();
        transcribe(&mut doc, item, "en", &words);
    }
    let found = script::find(&doc, &[one, two], &lines);
    assert_eq!(found[0].len(), 2, "two takes of the first line");
    assert_eq!(
        (found[0][0].item, found[0][0].from, found[0][0].to),
        (one, 1, 6)
    );
    assert_eq!(found[0][1].item, two);
    assert!(found[0][0].score > found[0][1].score);
    assert_eq!((found[1][0].from, found[1][0].to), (6, 10));
    assert!(found[2].is_empty() && found[3].is_empty());
}

/// **A two-microphone interview.** The silence between one speaker and the
/// other, each on a row of their own, is a pause, since speech plays on
/// under it, so shortening pauses reaches it. A b-roll cut on a silent
/// picture clip is no edit point between words. And a word both microphones
/// heard, taken out of both, comes back once.
#[test]
fn two_microphones_pause_between_speakers_and_restore_once() {
    let mut doc = Document::new();
    let (ann, bo) = (footage(&mut doc), footage(&mut doc));
    transcribe(
        &mut doc,
        ann,
        "en",
        &[("Ready?", 0, 500, 0.9), ("Yes.", 3000, 3300, 0.4)],
    );
    transcribe(
        &mut doc,
        bo,
        "en",
        &[("Yes.", 3000, 3300, 0.8), ("Go.", 3400, 3800, 0.9)],
    );
    let c = comp(&mut doc, 25);
    // Each microphone lifted from 1 s to 3.36 s, so both lose the "Yes.".
    row(
        &mut doc,
        c,
        vec![clip(ann, 0, 1000, 0), clip(ann, 3360, 4000, 1000)],
    );
    row(
        &mut doc,
        c,
        vec![clip(bo, 0, 1000, 0), clip(bo, 3360, 4000, 1000)],
    );
    // B-roll cut at 0.8 s on a heard picture row, its own sound off.
    let mut broll = Clip::new(
        ClipSource::Solid(Uuid::now_v7()),
        ms(0),
        ms(800),
        ms(0),
        ms(800),
    );
    broll.audible = false;
    let mut after = clip(ann, 0, 840, 800);
    after.audible = false;
    layer(
        &mut doc,
        c,
        LayerKind::Sequence {
            clips: vec![broll, after],
        },
        false,
    );

    let look = Look {
        show_removed: true,
        ..Look::default()
    };
    let tokens = tokens(&doc, doc.comp(c).unwrap(), &look);
    let kinds: Vec<TokenKind> = tokens.iter().map(|t| t.kind).collect();
    use TokenKind::{Pause, Removed, Word};
    assert_eq!(kinds, [Word, Pause, Removed, Word]);
    assert_eq!(
        tokens[1].clip, tokens[0].clip,
        "the pause is the word before's"
    );
    // Its silence is 0.5 s to 1.04 s, with 100 ms kept either side.
    assert_eq!(
        shorten_pauses(&tokens, ms(500), ms(200), FrameRate::new(25, 1).unwrap()),
        [(ms(600), ms(920))]
    );
    assert!(
        !tokens[1].edit_before,
        "the b-roll cut is not an edit point here"
    );
    let restored = tokens[2].word.unwrap();
    assert_eq!(
        (restored.item, restored.index),
        (bo, 0),
        "the surer of the two"
    );
}

/// **Speech on another row.** A silence is measured from the latest end of
/// any word, so a short word on one row said inside a long word on another
/// leaves no pause in the long word for shortening to cut, and sets off no
/// phrase after it. The short word takes no time, so deleting it takes
/// nothing of the long word.
#[test]
fn a_pause_never_lies_over_a_word_still_playing() {
    let mut doc = Document::new();
    let (a, b) = (footage(&mut doc), footage(&mut doc));
    transcribe(
        &mut doc,
        a,
        "en",
        &[("Well", 0, 1200, 0.9), ("so", 1300, 1600, 0.9)],
    );
    transcribe(&mut doc, b, "en", &[("yeah", 200, 400, 0.9)]);
    let c = comp(&mut doc, 25);
    row(&mut doc, c, vec![clip(a, 0, 2000, 0)]);
    row(&mut doc, c, vec![clip(b, 0, 2000, 0)]);
    let tokens = tokens(&doc, doc.comp(c).unwrap(), &Look::default());
    let rate = FrameRate::new(25, 1).unwrap();
    assert!(shorten_pauses(&tokens, ms(500), ms(300), rate).is_empty());
    let laid: Vec<(Rational, Rational)> = tokens.iter().map(|t| (t.start, t.end)).collect();
    assert_eq!(
        laid,
        [
            (ms(0), ms(1240)),
            (ms(1240), ms(1240)),
            (ms(1240), ms(2000))
        ]
    );
    assert_eq!(tokens[2].filler, None, "\"so\" is not set off");
}

/// **Retakes.** A sentence the speaker stopped and said again soon after is
/// marked, every word of the earlier attempt, and the attempt kept and the
/// apology between them are not. Two words said twice are too short to
/// count.
#[test]
fn an_earlier_attempt_at_a_sentence_is_a_retake() {
    let script = "Start with sound. People forgive a soft picture, but they won't forgive bad sound. \
                  Sorry, let me say that again. \
                  Start with sound. People forgive a soft picture, but they never forgive bad sound. \
                  Thank you. Thank you.";
    let ann = Uuid::now_v7();
    let mut words: Vec<Word> = Vec::new();
    for (i, text) in script.split_whitespace().enumerate() {
        let at = i as i64 * 400;
        let mut word = Word::new(text, SourceTime(ms(at)), SourceTime(ms(at + 300)), 0.9);
        word.speaker = Some(ann);
        words.push(word);
    }
    let count = words.len() as i64;
    let mut doc = Document::new();
    let item = footage(&mut doc);
    doc.transcripts.insert(
        item,
        Arc::new(Transcript {
            language: "en".into(),
            stream: 0,
            words,
            speakers: vec![crate::transcript::Speaker {
                id: ann,
                name: "Ann".into(),
            }],
            model: String::new(),
            extra: serde_json::Map::new(),
        }),
    );
    let c = comp(&mut doc, 25);
    row(&mut doc, c, vec![clip(item, 0, count * 400, 0)]);
    let marked: Vec<(String, Option<Filler>)> =
        tokens(&doc, doc.comp(c).unwrap(), &Look::default())
            .iter()
            .filter_map(|t| {
                let w = t.word?;
                Some((doc.transcripts[&item].words[w.index].text.clone(), t.filler))
            })
            .collect();
    let retake = |range: std::ops::Range<usize>| {
        marked[range]
            .iter()
            .all(|(_, f)| *f == Some(Filler::Retake))
    };
    assert!(retake(0..14), "the first attempt, both sentences");
    let rest = &marked[14..];
    assert!(
        rest.iter().all(|(_, f)| *f != Some(Filler::Retake)),
        "the apology, the kept attempt and the thanks: {rest:?}"
    );
}

/// The filler marked on each word a cut says, in order. Each row plays its
/// own file, its words as (text, start ms, end ms).
fn fillers_said(language: &str, rows: &[Vec<(&str, i64, i64)>]) -> Vec<Option<Filler>> {
    let mut doc = Document::new();
    let c = comp(&mut doc, 25);
    for words in rows {
        let item = footage(&mut doc);
        let words: Vec<_> = words.iter().map(|(w, a, b)| (*w, *a, *b, 0.9)).collect();
        transcribe(&mut doc, item, language, &words);
        row(&mut doc, c, vec![clip(item, 0, 3000, 0)]);
    }
    tokens(&doc, doc.comp(c).unwrap(), &Look::default())
        .iter()
        .filter_map(|t| t.word.map(|_| t.filler))
        .collect()
}

/// **Words that are not fillers.** A phrase is set off on both sides, so
/// "well," before a comma and "É" after a pause stay. Two people on two rows
/// saying one word is no restart, even with no speakers told apart.
#[test]
fn a_filler_is_set_off_on_both_sides_and_said_by_one_voice() {
    let spaced = |text: &'static str| -> Vec<(&str, i64, i64)> {
        text.split(' ')
            .enumerate()
            .map(|(i, w)| (w, i as i64 * 300, i as i64 * 300 + 250))
            .collect()
    };
    let well = fillers_said("en", &[spaced("We went there as well, and it rained.")]);
    assert!(well.iter().all(Option::is_none), "{well:?}");
    let pt = vec![
        ("Ele", 0, 200),
        ("é", 300, 500),
        ("médico.", 600, 900),
        ("É", 1500, 1700),
        ("cirurgião.", 1800, 2300),
    ];
    let pt = fillers_said("pt", &[pt]);
    assert!(pt.iter().all(Option::is_none), "{pt:?}");
    let two = fillers_said("en", &[vec![("Yeah", 0, 300)], vec![("yeah", 500, 800)]]);
    assert_eq!(two, [None, None]);
}

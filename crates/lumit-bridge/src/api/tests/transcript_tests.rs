//! Editing a cut by its words: the Transcript panel's edits reach the media
//! they stand for, in one undo step each.

use std::sync::Arc;

use super::*;
use crate::api::cut::transcript::{
    BridgeFrameSpan, BridgeRemovedRun, BridgeSpoken, BridgeSpokenKind, BridgeSpokenLook,
    BridgeWordRun, BridgeWordsEdit,
};
use crate::api::cut::BridgeCutResult;
use lumit_core::sequence::Clip;
use lumit_core::time::{Rational, SourceTime};
use lumit_core::transcript::{Speaker, Transcript, TranscriptEdit, Word};

const LOOK: BridgeSpokenLook = BridgeSpokenLook {
    pause_ms: 500,
    show_removed: true,
};

/// Five words with a long silence after the second, on a 4 s clip of a file
/// on one picture row: the comp, the row, the footage.
fn spoken_row() -> (
    ProjectReference,
    CompositionReference,
    LayerReference,
    FootageReference,
) {
    let project = LumitBridgeState::new_project(None).expect("a new project");
    let comp = project.new_composition("Scene".into(), None).expect("comp");
    let footage = project
        .import_footage("C:/clips/talk.mov".into())
        .expect("imported");
    let ms = |n: i64| SourceTime(Rational::new(n, 1000).expect("ms"));
    let words = [
        ("We", 0, 300),
        ("went", 400, 800),
        ("home", 2000, 2400),
        ("very", 2500, 2900),
        ("late.", 3000, 3600),
    ]
    .map(|(text, a, b)| Word::new(text, ms(a), ms(b), 0.9));
    let transcript = Transcript {
        language: "en".into(),
        stream: 0,
        words: words.to_vec(),
        speakers: vec![Speaker {
            id: Uuid::now_v7(),
            name: "Ann".into(),
        }],
        model: String::new(),
        extra: serde_json::Map::new(),
    };
    comp.commit(Op::EditTranscript {
        item: footage.id(),
        edit: Box::new(TranscriptEdit::Set(Some(Arc::new(transcript)))),
    })
    .expect("transcribed");
    let span = |n: i64| BridgeRational { num: n, den: 1 };
    let done = comp.cut_place(
        None,
        &footage,
        span(0),
        span(4),
        0,
        false,
        true,
        false,
        None,
    );
    assert_eq!(done.expect("placed"), BridgeCutResult::Done);
    let row = comp.get_layers().expect("layers").remove(0);
    (project, comp, row, footage)
}

fn clips(row: &LayerReference) -> Vec<Clip> {
    let lumit_core::model::LayerKind::Sequence { mut clips } = row.item().expect("row").kind else {
        panic!("a Sequence layer");
    };
    clips.sort_by_key(|c| c.place_start);
    clips
}

/// The token saying `text`.
fn word<'a>(tokens: &'a [BridgeSpoken], text: &str) -> &'a BridgeSpoken {
    tokens
        .iter()
        .find(|t| t.text == text && t.kind == BridgeSpokenKind::Word)
        .expect("the word")
}

fn span(from: &BridgeSpoken, to: &BridgeSpoken) -> BridgeFrameSpan {
    BridgeFrameSpan {
        start_frame: from.start_frame,
        end_frame: to.end_frame,
    }
}

/// **Deleting words deletes their media.** An extract cuts the row at the
/// words' edges and closes it, so the clip after starts on the source the
/// next word starts on; one undo puts it back. The words taken out show at
/// the edit point and come back with Restore. A lift leaves the gap.
#[test]
fn deleting_words_cuts_their_media_and_restore_puts_it_back() {
    let (project, comp, row, footage) = spoken_row();
    let read = || comp.cut_transcript(LOOK).expect("read").tokens;
    let tokens = read();
    let rate = comp.composition().expect("comp").frame_rate;
    let at = |frame: i64| rate.time_of_frame(frame).expect("frame").0;
    let (went, home) = (word(&tokens, "went").clone(), word(&tokens, "home").clone());
    assert!(
        !tokens[0].space_before && went.space_before,
        "a space between words"
    );

    let done = comp.cut_transcript_remove(vec![span(&went, &home)], true);
    assert_eq!(done.expect("removed"), BridgeCutResult::Done);
    let kept = clips(&row);
    assert_eq!(kept.len(), 2);
    assert_eq!(kept[0].place_end(), at(went.start_frame));
    assert_eq!(kept[1].place_start, at(went.start_frame), "closed up");
    assert_eq!(
        kept[1].source_in,
        at(home.end_frame),
        "on the next word's source"
    );
    let after = read();
    let gone: Vec<&str> = after
        .iter()
        .filter(|t| t.kind == BridgeSpokenKind::Removed)
        .map(|t| t.text.as_str())
        .collect();
    assert_eq!(gone, ["went", "home"]);

    // Restore the two words, each as its own run and the earlier one named
    // first: the clip after makes room for both.
    let run = |word: u32| BridgeRemovedRun {
        clip: kept[0].id.to_string(),
        item: footage.clone(),
        from: word,
        to: word,
    };
    let done = comp.cut_transcript_restore(vec![run(1), run(2)]);
    assert_eq!(done.expect("restored"), BridgeCutResult::Done);
    let back = read();
    assert!(back.iter().all(|t| t.kind != BridgeSpokenKind::Removed));
    assert_eq!(clips(&row).len(), 4);

    project.undo().expect("undo");
    project.undo().expect("undo");
    assert_eq!(clips(&row).len(), 1, "each edit one step");

    let done = comp.cut_transcript_remove(vec![span(&went, &home)], false);
    assert_eq!(done.expect("lifted"), BridgeCutResult::Done);
    let lifted = clips(&row);
    assert_eq!(lifted[1].place_start, at(home.end_frame), "the gap stays");
}

/// **Copy, paste and drag.** A copy pasted at a word's edge opens the cut by
/// its length and plays the copied source there, and a drag without copy
/// moves the words: the cut keeps its length and says them later.
#[test]
fn words_copy_paste_and_drag() {
    let (project, comp, row, _) = spoken_row();
    let tokens = comp.cut_transcript(LOOK).expect("read").tokens;
    let rate = comp.composition().expect("comp").frame_rate;
    let at = |frame: i64| rate.time_of_frame(frame).expect("frame").0;
    let (we, late) = (word(&tokens, "We").clone(), word(&tokens, "late.").clone());
    let length = clips(&row)[0].place_duration;

    let done = comp.cut_transcript_copy(vec![span(&we, &we)]);
    assert_eq!(done.expect("copied"), BridgeCutResult::Done);
    let pasted = comp.cut_transcript_paste(late.start_frame).expect("pasted");
    assert_eq!(pasted.result, BridgeCutResult::Done);
    let laid = clips(&row);
    assert_eq!(laid.len(), 3);
    assert_eq!(laid[1].place_start, at(late.start_frame));
    assert_eq!(laid[1].source_in, Rational::ZERO, "the copied word");
    assert_eq!(
        pasted.end_frame,
        late.start_frame + (we.end_frame - we.start_frame)
    );
    project.undo().expect("undo");

    let moved = comp
        .cut_transcript_move(vec![span(&we, &we)], late.start_frame, false)
        .expect("moved");
    assert_eq!(moved.result, BridgeCutResult::Done);
    let laid = clips(&row);
    let total = laid
        .iter()
        .map(|c| c.place_duration)
        .fold(Rational::ZERO, |a, b| a.checked_add(b).expect("sum"));
    assert_eq!(total, length, "nothing gained or lost");
    assert_eq!(
        laid[0].source_in,
        at(we.end_frame),
        "it starts on the next word"
    );
    assert!(laid
        .iter()
        .any(|c| c.source_in == Rational::ZERO && c.place_start > Rational::ZERO));
    let into = comp.cut_transcript_move(vec![span(&we, &late)], we.end_frame, false);
    assert_eq!(into.expect("answered").result, BridgeCutResult::Nothing);
}

/// **Pauses shorten in their middle, and corrections touch only words.**
/// The long silence keeps what was asked for of it. A correction to the same
/// number of words keeps their times, and to another number spreads them
/// over the same stretch; a speaker given to a run lands on those words. The
/// media is not touched by either, and each is one undo step.
#[test]
fn pauses_shorten_and_corrections_keep_the_media() {
    let (project, comp, row, footage) = spoken_row();
    let tokens = comp.cut_transcript(LOOK).expect("read").tokens;
    let pause = tokens
        .iter()
        .find(|t| t.kind == BridgeSpokenKind::Pause)
        .expect("a pause");
    let (went, home) = (word(&tokens, "went"), word(&tokens, "home"));
    assert_eq!(
        (pause.heard_start_frame, pause.heard_end_frame),
        (went.heard_end_frame, home.heard_start_frame),
        "a pause is heard as its whole silence"
    );
    let done = comp.cut_shorten_pauses(LOOK, 500, 300, None, true);
    assert_eq!(done.expect("shortened"), BridgeCutResult::Done);
    let tokens = comp.cut_transcript(LOOK).expect("read").tokens;
    let (went, home) = (word(&tokens, "went"), word(&tokens, "home"));
    let rate = comp.composition().expect("comp").frame_rate;
    let silence = rate
        .time_of_frame(home.heard_start_frame - went.heard_end_frame + 1)
        .expect("time")
        .0;
    assert!(silence <= Rational::new(2, 5).expect("400 ms"));
    assert!(
        silence >= Rational::new(3, 10).expect("300 ms"),
        "what was asked for kept, less a frame"
    );
    project.undo().expect("undo");
    let media = clips(&row);

    let item = footage.clone();
    project
        .correct_words(vec![
            BridgeWordsEdit {
                item: item.clone(),
                at: 2,
                remove: 1,
                texts: vec!["house".into()],
            },
            BridgeWordsEdit {
                item: item.clone(),
                at: 3,
                remove: 2,
                texts: vec!["so".into(), "very".into(), "late.".into()],
            },
        ])
        .expect("corrected");
    let speaker = {
        let doc = comp.document().expect("doc");
        doc.transcripts[&item.id()].speakers[0].id.to_string()
    };
    project
        .set_spoken_speaker(
            vec![BridgeWordRun {
                item: item.clone(),
                from: 0,
                to: 1,
            }],
            Some(speaker),
        )
        .expect("named");
    let doc = comp.document().expect("doc");
    let words = &doc.transcripts[&item.id()].words;
    let texts: Vec<&str> = words.iter().map(|w| w.text.as_str()).collect();
    assert_eq!(texts, ["We", "went", "house", "so", "very", "late."]);
    assert_eq!(
        words[2].start.0,
        Rational::new(2, 1).expect("2 s"),
        "the same times"
    );
    assert_eq!(words[3].start.0, Rational::new(5, 2).expect("2.5 s"));
    assert_eq!(
        words[5].end.0,
        Rational::new(18, 5).expect("3.6 s"),
        "the same stretch"
    );
    assert!(words[0].speaker.is_some() && words[2].speaker.is_none());
    assert_eq!(clips(&row), media, "the media untouched");
    project.undo().expect("undo");
    project.undo().expect("undo");
    let doc = comp.document().expect("doc");
    let texts: Vec<&str> = doc.transcripts[&item.id()]
        .words
        .iter()
        .map(|w| w.text.as_str())
        .collect();
    assert_eq!(
        texts,
        ["We", "went", "home", "very", "late."],
        "one step each"
    );
}

/// **A Footage layer converts when its words are cut.** Its words are heard
/// as they are, and deleting one makes it a Sequence layer with the same id,
/// in the same undo step as the cut.
#[test]
fn a_footage_layer_becomes_a_sequence_layer_when_its_words_are_cut() {
    let (project, comp, row, footage) = spoken_row();
    comp.commit(Op::RemoveLayer {
        comp: comp.id(),
        layer: row.id(),
    })
    .expect("row gone");
    comp.add_audio_layer(&footage, None).expect("placed");
    let layer = comp.get_layers().expect("layers").remove(0);
    let tokens = comp.cut_transcript(LOOK).expect("read").tokens;
    let home = word(&tokens, "home").clone();
    let done = comp.cut_transcript_remove(vec![span(&home, &home)], true);
    assert_eq!(done.expect("removed"), BridgeCutResult::Done);
    let cut = clips(&layer);
    assert_eq!(cut.len(), 2, "made a Sequence layer and cut");
    let click = Rational::new(1, 500).expect("2 ms");
    assert_eq!(
        (cut[0].fade_out.seconds, cut[1].fade_in.seconds),
        (click, click),
        "no click"
    );
    project.undo().expect("undo");
    assert!(matches!(
        layer.item().expect("layer").kind,
        lumit_core::model::LayerKind::Footage { .. }
    ));
}

/// **A cut from a script, and from a source's words.** Each line said goes
/// down in the script's order as one clip from its words, its other take
/// held on the clip, and a line nobody said comes back missing. Words picked in
/// a source go down one after another, in one undo step.
#[test]
fn a_script_and_a_source_selection_lay_down_a_cut() {
    let (project, comp, row, footage) = spoken_row();
    // Another take of the first line, said less well.
    let other = project
        .import_footage("C:/clips/take2.mov".into())
        .expect("imported");
    let ms = |n: i64| SourceTime(Rational::new(n, 1000).expect("ms"));
    let words = [("very", 0, 300), ("much", 400, 700), ("late.", 800, 1200)]
        .map(|(text, a, b)| Word::new(text, ms(a), ms(b), 0.9));
    let take = Transcript {
        language: "en".into(),
        stream: 0,
        words: words.to_vec(),
        speakers: Vec::new(),
        model: String::new(),
        extra: serde_json::Map::new(),
    };
    comp.commit(Op::EditTranscript {
        item: other.id(),
        edit: Box::new(TranscriptEdit::Set(Some(Arc::new(take)))),
    })
    .expect("transcribed");
    let found = project
        .find_script(
            "ANN
Very late.
We went.
(beat)
Nobody says this."
                .into(),
            vec![footage.clone(), other.clone()],
        )
        .expect("found");
    let assembled = project
        .assemble_from_script(found, None, 0, "Assembly".into())
        .expect("assembled");
    assert_eq!(assembled.result, BridgeCutResult::Done);
    assert_eq!(assembled.placed, 2);
    assert_eq!(assembled.missing, ["Nobody says this."]);
    let made = assembled.comp.expect("a new composition");
    let laid = clips(&made.get_layers().expect("layers").remove(0));
    assert_eq!(laid.len(), 2);
    assert_eq!(
        laid[0].source_in,
        ms(2350).0,
        "the first line's words, a lead before"
    );
    assert_eq!(laid[0].takes.len(), 1, "the other take held on the clip");
    assert_eq!(laid[1].takes.len(), 0);
    assert_eq!(laid[1].source_in, Rational::ZERO, "in the script's order");
    project.undo().expect("undo");
    assert!(
        comp.document().expect("doc").comp(made.id()).is_none(),
        "one step"
    );

    let half = |n: i64| BridgeRational { num: n, den: 2 };
    let spans = vec![
        crate::api::cut::transcript::BridgeSourceSpan {
            start: half(5),
            end: half(7),
        },
        crate::api::cut::transcript::BridgeSourceSpan {
            start: half(0),
            end: half(1),
        },
    ];
    let before = clips(&row).len();
    let placed = comp
        .cut_place_words(Some(row), &footage, spans, 0, true, true, false, None)
        .expect("placed");
    assert_eq!(placed.result, BridgeCutResult::Done);
    let laid = clips(&row);
    assert_eq!(laid.len(), before + 2);
    assert_eq!(laid[0].source_in, Rational::new(5, 2).expect("2.5 s"));
    assert_eq!(laid[1].place_start, Rational::ONE, "one after another");
    project.undo().expect("undo");
    assert_eq!(clips(&row).len(), before);
}

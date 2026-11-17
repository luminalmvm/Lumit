//! Captions made from what a layer says: they land on its words, an edit keeps
//! the words it does not touch, a subtitle file says the same, and each step
//! is one undo.

use super::*;
use crate::api::{
    captions::{BridgeCaptionFormat, BridgeCaptionOptions},
    transcript::{BridgeTranscript, BridgeWord},
};

fn word(text: &str, from: i64, to: i64) -> BridgeWord {
    BridgeWord {
        text: text.into(),
        start: BridgeRational {
            num: from,
            den: 1000,
        },
        end: BridgeRational { num: to, den: 1000 },
        confidence: 0.9,
        speaker: None,
        paragraph: false,
    }
}

fn texts(layer: &LayerReference) -> Vec<String> {
    let captions = layer.captions().expect("read").expect("a caption layer");
    captions.cues.iter().map(|c| c.text.clone()).collect()
}

fn seconds(at: Option<BridgeRational>) -> f64 {
    let at = at.expect("in the edit");
    at.num as f64 / at.den as f64
}

/// **Captions from a transcript, edited and written out.** A footage layer's
/// words become two captions split at the pause between them, at the moments
/// they are said. Retyping keeps the times of the words it keeps, splitting
/// and joining lose no word, find and replace changes only whole words, the
/// subtitle file says what the layer shows, a split at the playhead splits
/// at the word there, and undo walks every step back.
/// Wrong here is words lost from somebody's captions, or a file that says
/// something else.
#[test]
fn captions_from_a_transcript_edit_and_write_out() {
    let project = LumitBridgeState::new_project(None).expect("a new project");
    let comp = project.new_composition("Talk".into(), None).expect("comp");
    let footage = project
        .import_footage("C:/clips/talk.mov".into())
        .expect("imported");
    footage
        .set_transcript(BridgeTranscript {
            language: "en".into(),
            stream: 0,
            model: "test".into(),
            speakers: Vec::new(),
            words: vec![
                word("Hello", 500, 900),
                word("there.", 950, 1400),
                word("How", 3000, 3200),
                word("are", 3250, 3400),
                word("you?", 3450, 3900),
            ],
        })
        .expect("transcribed");
    comp.add_footage_layer(&footage, false, None, None)
        .expect("placed");
    let source = comp.get_layers().expect("layers").remove(0);
    assert_eq!(source.heard_items().expect("heard").len(), 1);
    let look = comp
        .add_text_layer(None)
        .expect("text")
        .get_text()
        .expect("read")
        .expect("a text layer");

    let options = BridgeCaptionOptions {
        limits: comp.caption_limits_for("en".into()).expect("limits"),
        name: "Captions".into(),
        fillers: false,
    };
    let layer = comp
        .create_captions(source, options, look)
        .expect("captions made");
    let made = texts(&layer);
    assert_eq!(made, ["Hello there.", "How are you?"]);
    let first = &layer.captions().unwrap().unwrap().cues[0];
    assert!((seconds(first.start) - 0.5).abs() < 1e-9);
    assert!((seconds(first.words[1].start) - 0.95).abs() < 1e-9);

    layer
        .set_cue_text(0, "Hello there, friend.".into())
        .expect("retyped");
    let retyped = &layer.captions().unwrap().unwrap().cues[0];
    assert_eq!(retyped.text, "Hello there, friend.");
    assert!(
        (seconds(retyped.words[0].start) - 0.5).abs() < 1e-9,
        "a word kept keeps its time"
    );

    layer.split_cue(1, 1).expect("split");
    assert_eq!(texts(&layer), ["Hello there, friend.", "How", "are you?"]);
    layer.merge_cues(1, 2).expect("joined");
    assert_eq!(texts(&layer), ["Hello there, friend.", "How are you?"]);
    let changed = layer
        .replace_in_captions("you".into(), "we".into(), false, true)
        .expect("replaced");
    assert_eq!(changed, 1);
    assert_eq!(texts(&layer), ["Hello there, friend.", "How are we?"]);

    let file = std::env::temp_dir().join(format!("lumit-captions-{}.srt", Uuid::now_v7()));
    layer
        .export_captions(
            BridgeCaptionFormat::Srt,
            file.to_string_lossy().into_owned(),
        )
        .expect("written");
    let written = std::fs::read_to_string(&file).expect("read back");
    let _ = std::fs::remove_file(&file);
    assert!(written.contains("00:00:00,500 --> "), "{written}");
    assert!(
        written.contains("Hello there, friend.") && written.contains("How are we?"),
        "{written}"
    );

    // Split at the playhead before the second of two words alike, then take
    // out the two halves in one step.
    layer
        .set_cue_text(1, "the end of the end".into())
        .expect("retyped");
    layer
        .split_cue_at(BridgeRational { num: 352, den: 100 })
        .expect("split at the playhead");
    assert_eq!(
        texts(&layer),
        ["Hello there, friend.", "the end of", "the end"]
    );
    layer.remove_cues(vec![2, 1]).expect("removed");
    assert_eq!(texts(&layer), ["Hello there, friend."]);

    for _ in 0..7 {
        project.undo().expect("undone");
    }
    assert_eq!(texts(&layer), made, "seven undos take seven edits back");
    project.undo().expect("undone");
    assert!(
        layer.captions().is_err(),
        "and one more takes the layer away"
    );
}

use super::*;

/// An unstyled layer writes nothing, and a styled one writes only what was
/// changed and reads back the same.
#[test]
fn a_style_writes_only_what_was_changed() {
    assert_eq!(serde_json::to_string(&TextStyle::default()).unwrap(), "{}");
    assert_eq!(
        serde_json::to_string(&ParagraphStyle::default()).unwrap(),
        "{}"
    );
    assert_eq!(
        serde_json::from_str::<TextStyle>("{}").unwrap(),
        TextStyle::default()
    );

    let style = TextStyle {
        family: "Arial".into(),
        face: "Bold".into(),
        leading: Some(90.0),
        kerning: Kerning::Metrics,
        tracking: 50.0,
        scale_x: 87.0,
        stroke_on: true,
        stroke_width: 4.0,
        ligatures: false,
        ..TextStyle::default()
    };
    let json = serde_json::to_string(&style).unwrap();
    assert!(!json.contains("scale_y"), "{json}");
    assert!(!json.contains("caps"), "{json}");
    assert_eq!(serde_json::from_str::<TextStyle>(&json).unwrap(), style);
    assert_ne!(style.key_bytes(), TextStyle::default().key_bytes());

    let paragraph = ParagraphStyle {
        align: TextAlign::Centre,
        space_after: 12.0,
        ..ParagraphStyle::default()
    };
    let json = serde_json::to_string(&paragraph).unwrap();
    assert!(!json.contains("indent"), "{json}");
    assert_eq!(
        serde_json::from_str::<ParagraphStyle>(&json).unwrap(),
        paragraph
    );
}

fn selector(start: f64, end: f64, offset: f64) -> RangeSelector {
    RangeSelector {
        start: Property::fixed(start),
        end: Property::fixed(end),
        offset: Property::fixed(offset),
        ..RangeSelector::default()
    }
}

/// Counting words, the letters of one word move **together** — which is the
/// whole difference between the two bases and the reason both exist.
#[test]
fn a_word_selector_moves_a_whole_word_at_once() {
    let mut a = TextAnimator::new("Word");
    a.selector = selector(0.0, 50.0, 0.0);
    a.selector.basis = SelectorBasis::Words;
    a.position_y = Property::fixed(-40.0);
    let x = glyph_xforms(&[a], "ab cd", 0.0, &[]);
    assert_eq!(x.len(), 5);
    // "ab " is the first of two words, and every one of its characters —
    // the space included — is pushed the same way.
    for c in &x[0..3] {
        assert!((c.position[1] + 40.0).abs() < 1e-4, "{c:?}");
    }
    for c in &x[3..5] {
        assert_eq!(c.position[1], 0.0, "the second word moved");
    }
}

/// **Two animators compose.** A fade and a drop written separately have to
/// arrive together on the letters both of them reach — pushes add, scales
/// and opacities multiply.
#[test]
fn two_animators_compose_rather_than_replace() {
    let mut drop = TextAnimator::new("Drop");
    drop.position_y = Property::fixed(-30.0);
    drop.scale_x = Property::fixed(50.0);
    drop.opacity = Property::fixed(50.0);
    let mut fade = TextAnimator::new("Fade");
    fade.position_y = Property::fixed(-10.0);
    fade.scale_x = Property::fixed(50.0);
    fade.opacity = Property::fixed(50.0);
    let x = glyph_xforms(&[drop, fade], "A", 0.0, &[]);
    assert!((x[0].position[1] + 40.0).abs() < 1e-4, "{x:?}");
    assert!((x[0].scale[0] - 0.25).abs() < 1e-6, "{x:?}");
    assert!((x[0].opacity - 0.25).abs() < 1e-6, "{x:?}");
}

/// A weight of a half moves a letter half as far — the property is applied
/// *times* the weight, which is what makes a ramp read as a sweep.
#[test]
fn a_half_weighted_letter_is_moved_half_as_far() {
    let mut a = TextAnimator::new("Half");
    a.selector = RangeSelector {
        shape: SelectorShape::Ramp,
        ..selector(0.0, 100.0, 0.0)
    };
    a.position_x = Property::fixed(100.0);
    a.scale_x = Property::fixed(200.0);
    let x = glyph_xforms(&[a], "ab", 0.0, &[]);
    // Two letters: their middles sit at 25 % and 75 % of the run.
    assert!((x[0].position[0] - 25.0).abs() < 1e-4, "{x:?}");
    assert!((x[1].position[0] - 75.0).abs() < 1e-4, "{x:?}");
    assert!((x[0].scale[0] - 1.25).abs() < 1e-6, "{x:?}");
}

/// **Spoken selectors follow the words as they are said.** Each mode weighs
/// a word by its own times: the one being said, the ones said, the ones to
/// come, the emphasised ones, letter by letter across a word, and eased in
/// over a word's first moments. No words, no letters reached. Wrong here is a
/// caption style highlighting the wrong word.
#[test]
fn spoken_selectors_follow_the_words_as_they_are_said() {
    let spoken = [
        SpokenWord {
            chars: 0..2,
            start: 1.0,
            end: 2.0,
            emphasis: false,
        },
        SpokenWord {
            chars: 3..5,
            start: 2.0,
            end: 3.0,
            emphasis: true,
        },
    ];
    let pushes = |mode: SelectorMode,
                  basis: SelectorBasis,
                  shape: SelectorShape,
                  ease: f64,
                  lt: f64|
     -> Vec<f32> {
        let mut a = TextAnimator::new("Said");
        a.selector.mode = mode;
        a.selector.basis = basis;
        a.selector.shape = shape;
        a.selector.ease = ease;
        a.position_y = Property::fixed(-10.0);
        glyph_xforms(&[a], "ab cd", lt, &spoken)
            .iter()
            .map(|x| -x.position[1] / 10.0)
            .collect()
    };
    let close = |got: Vec<f32>, want: [f32; 5]| {
        let same =
            got.len() == want.len() && got.iter().zip(want).all(|(g, w)| (g - w).abs() < 1e-4);
        assert!(same, "{got:?}, not {want:?}");
    };
    let words = SelectorBasis::Words;
    let step = SelectorShape::Square;
    close(
        pushes(SelectorMode::Current, words, step, 0.0, 1.5),
        [1.0, 1.0, 1.0, 0.0, 0.0],
    );
    close(pushes(SelectorMode::Said, words, step, 0.0, 2.5), [1.0; 5]);
    close(
        pushes(SelectorMode::Unsaid, words, step, 0.0, 1.5),
        [0.0, 0.0, 0.0, 1.0, 1.0],
    );
    close(
        pushes(SelectorMode::Emphasis, words, step, 0.0, 0.0),
        [0.0, 0.0, 0.0, 1.0, 1.0],
    );
    // Letter by letter, "b" is said halfway through its word, and the space,
    // in no word, counts as said.
    let letters = SelectorBasis::Characters;
    close(
        pushes(SelectorMode::Said, letters, step, 0.0, 1.4),
        [1.0, 0.0, 1.0, 0.0, 0.0],
    );
    // Eased over half a second: a quarter second into "cd", each word is
    // halfway between being said and not.
    let eased = pushes(
        SelectorMode::Current,
        words,
        SelectorShape::Smooth,
        0.5,
        2.25,
    );
    for w in eased {
        assert!((w - 0.5).abs() < 1e-4, "{w}");
    }
    // A bounce overshoots on its way in.
    assert!(pushes(SelectorMode::Said, words, SelectorShape::Bounce, 1.0, 1.5)[0] > 1.0);
    assert!(pushes(SelectorMode::Said, words, SelectorShape::Ramp, 1.0, 1.5)[0] > 0.4);

    let mut a = TextAnimator::new("Said");
    a.selector.mode = SelectorMode::Said;
    a.position_y = Property::fixed(-10.0);
    for x in glyph_xforms(&[a], "ab cd", 2.5, &[]) {
        assert_eq!(x.position[1], 0.0, "no words reach no letter");
    }
}

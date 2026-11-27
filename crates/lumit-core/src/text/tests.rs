#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::selector::{near_weight, SelectorShape};
use super::*;

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

#[test]
fn a_word_selector_moves_a_whole_word_at_once() {
    let mut a = TextAnimator::new("Word");
    a.selector = selector(0.0, 50.0, 0.0);
    a.selector.basis = SelectorBasis::Words;
    a.position_y = Property::fixed(-40.0);
    let x = glyph_xforms(&[a], "ab cd", 0.0, &[]);
    assert_eq!(x.len(), 5);
    for c in &x[0..3] {
        assert!((c.position[1] + 40.0).abs() < 1e-4, "{c:?}");
    }
    for c in &x[3..5] {
        assert_eq!(c.position[1], 0.0, "the second word moved");
    }
}

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
    assert!((x[0].position[0] - 25.0).abs() < 1e-4, "{x:?}");
    assert!((x[1].position[0] - 75.0).abs() < 1e-4, "{x:?}");
    assert!((x[0].scale[0] - 1.25).abs() < 1e-6, "{x:?}");
}

/// **Subtract** turns a Range selector over: the letters its range reaches
/// are left alone and the others are moved, as After Effects' Mode Subtract.
#[test]
fn a_subtracting_range_moves_the_letters_it_does_not_reach() {
    let mut s = RangeSelector {
        shape: SelectorShape::Ramp,
        ..selector(0.0, 100.0, 0.0)
    };
    let add = s.weights_at(4, 0.0);
    s.invert = true;
    let subtract = s.weights_at(4, 0.0);
    for (a, b) in add.iter().zip(&subtract) {
        assert!((a + b - 1.0).abs() < 1e-6, "{add:?} {subtract:?}");
    }
    assert!(subtract[0] > subtract[3], "{subtract:?}");
}

/// **Ease High and Ease Low** bend a ramp: 0 % keeps it even, 100 % flattens
/// that end, −100 % steepens it; the ends stay at nothing and the whole way.
#[test]
fn ease_high_and_low_bend_the_ramp() {
    let mut s = RangeSelector {
        shape: SelectorShape::Ramp,
        ..selector(0.0, 100.0, 0.0)
    };
    let even = s.weights_at(10, 0.0);
    s.ease_low = Property::fixed(100.0);
    let eased_in = s.weights_at(10, 0.0);
    assert!(eased_in[1] < even[1], "{even:?} {eased_in:?}");
    s.ease_low = Property::zero();
    s.ease_high = Property::fixed(-50.0);
    let steep_top = s.weights_at(10, 0.0);
    // An upright handle reaches the whole way steeply: the last step is
    // bigger than the even ramp's.
    assert!(
        steep_top[9] - steep_top[8] > even[9] - even[8],
        "{even:?} {steep_top:?}"
    );
    assert!(
        steep_top.iter().all(|w| (0.0..=1.0).contains(w)),
        "{steep_top:?}"
    );
    // Past the range the letters are moved the whole way, eased or not.
    s.offset = Property::fixed(-100.0);
    assert!(s.weights_at(4, 0.0).iter().all(|w| (*w - 1.0).abs() < 1e-6));
}

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
    let letters = SelectorBasis::Characters;
    close(
        pushes(SelectorMode::Said, letters, step, 0.0, 1.4),
        [1.0, 0.0, 1.0, 0.0, 0.0],
    );
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
    assert!(pushes(SelectorMode::Said, words, SelectorShape::Bounce, 1.0, 1.5)[0] > 1.0);
    assert!(pushes(SelectorMode::Said, words, SelectorShape::Ramp, 1.0, 1.5)[0] > 0.4);

    let mut a = TextAnimator::new("Said");
    a.selector.mode = SelectorMode::Said;
    a.position_y = Property::fixed(-10.0);
    for x in glyph_xforms(&[a], "ab cd", 2.5, &[]) {
        assert_eq!(x.position[1], 0.0, "no words reach no letter");
    }
}

fn proximity(radius: f64, shape: SelectorShape, target: uuid::Uuid) -> TextAnimator {
    TextAnimator {
        selector: RangeSelector {
            kind: SelectorKind::Proximity,
            target: Some(target),
            radius: Property::fixed(radius),
            shape,
            ..RangeSelector::default()
        },
        position_y: Property::fixed(-10.0),
        ..TextAnimator::new("Near")
    }
}

#[test]
fn a_proximity_selector_reaches_the_letters_near_its_target() {
    let target = uuid::Uuid::from_u128(7);
    let near = Near {
        middles: vec![
            Some([0.0, 0.0]),
            Some([50.0, 0.0]),
            Some([100.0, 0.0]),
            None,
        ],
        targets: vec![(target, [0.0, 0.0])],
    };
    let ramp = glyph_xforms_near(
        &[proximity(100.0, SelectorShape::Ramp, target)],
        "abc\n",
        0.0,
        &[],
        &near,
    );
    let pushes: Vec<f32> = ramp.iter().map(|x| x.position[1]).collect();
    assert_eq!(pushes, vec![-10.0, -5.0, 0.0, 0.0]);

    let square = glyph_xforms_near(
        &[proximity(60.0, SelectorShape::Square, target)],
        "abc\n",
        0.0,
        &[],
        &near,
    );
    let pushes: Vec<f32> = square.iter().map(|x| x.position[1]).collect();
    assert_eq!(pushes, vec![-10.0, -10.0, 0.0, 0.0]);

    let smooth = near_weight(SelectorShape::Smooth, 25.0, 100.0);
    assert!((near_weight(SelectorShape::Smooth, 50.0, 100.0) - 0.5).abs() < 1e-6);
    assert!(smooth > 0.75 && smooth < 1.0, "{smooth}");

    let mut inverted = proximity(100.0, SelectorShape::Ramp, target);
    inverted.selector.invert = true;
    let x = glyph_xforms_near(&[inverted], "abc", 0.0, &[], &near);
    assert_eq!(x[0].position[1], 0.0);
    assert_eq!(x[2].position[1], -10.0);
}

#[test]
fn a_proximity_selector_without_its_target_reaches_nothing() {
    let animator = proximity(100.0, SelectorShape::Ramp, uuid::Uuid::from_u128(1));
    let x = glyph_xforms(&[animator], "ab", 0.0, &[]);
    assert!(x.iter().all(GlyphXform::is_identity), "{x:?}");
}

#[test]
fn a_font_axis_is_set_half_way_by_half_a_weight() {
    let mut a = TextAnimator::new("Weight");
    a.selector.shape = SelectorShape::Ramp;
    a.axes.push(AnimatorAxis {
        tag: "wght".into(),
        value: Property::fixed(900.0),
    });
    let x = glyph_xforms(&[a.clone()], "ab", 0.0, &[]);
    let base = |_: [u8; 4]| 100.0f32;
    let at = |x: &GlyphXform| x.axes.resolved(&base).next().unwrap();
    assert_eq!(at(&x[0]).0, *b"wght");
    assert!((at(&x[0]).1 - 300.0).abs() < 1e-3, "{:?}", at(&x[0]));
    assert!((at(&x[1]).1 - 700.0).abs() < 1e-3, "{:?}", at(&x[1]));

    let mut b = TextAnimator::new("Back");
    b.axes.push(AnimatorAxis {
        tag: "wght".into(),
        value: Property::fixed(500.0),
    });
    let x = glyph_xforms(&[a, b], "ab", 0.0, &[]);
    assert!((at(&x[0]).1 - 500.0).abs() < 1e-3);
    assert_eq!(x[0].axes.len, 1);
}

#[test]
fn a_layer_moves_at_most_eight_axes() {
    let mut a = TextAnimator::new("Many");
    for tag in [
        "AAAA", "BBBB", "CCCC", "DDDD", "EEEE", "FFFF", "GGGG", "HHHH", "IIII", "toolong",
    ] {
        a.axes.push(AnimatorAxis {
            tag: tag.into(),
            value: Property::fixed(1.0),
        });
    }
    let x = glyph_xforms(&[a], "a", 0.0, &[]);
    assert_eq!(usize::from(x[0].axes.len), MAX_ANIMATED_AXES);
    assert_eq!(axis_tag("wdth"), Some(*b"wdth"));
    assert_eq!(axis_tag("ab"), Some(*b"ab  "));
    assert_eq!(axis_tag(""), None);
}

#[test]
fn axes_and_proximity_write_only_what_was_set() {
    let plain = TextAnimator::new("A");
    let json = serde_json::to_value(&plain).unwrap();
    assert!(
        json.get("axes").is_none() && json.get("selector").is_none(),
        "{json}"
    );
    let mut set = proximity(150.0, SelectorShape::Smooth, uuid::Uuid::from_u128(9));
    set.axes.push(AnimatorAxis {
        tag: "wdth".into(),
        value: Property::fixed(75.0),
    });
    let back: TextAnimator = serde_json::from_str(&serde_json::to_string(&set).unwrap()).unwrap();
    assert_eq!(back, set);

    let style = TextStyle {
        axes: vec![FontAxisValue {
            tag: "wght".into(),
            value: 650.0,
        }],
        axis_spacing: AxisSpacing::PerCharacter,
        ..TextStyle::default()
    };
    let back: TextStyle = serde_json::from_str(&serde_json::to_string(&style).unwrap()).unwrap();
    assert_eq!(back, style);
}

#[test]
fn a_wiggly_selector_wanders_between_its_amounts() {
    let mut s = RangeSelector {
        kind: SelectorKind::Wiggly,
        min_amount: Property::fixed(20.0),
        max_amount: Property::fixed(80.0),
        correlation: Property::fixed(0.0),
        ..RangeSelector::default()
    };
    let a = s.wiggly_weights(6, 0.3);
    let b = s.wiggly_weights(6, 1.7);
    assert!(
        a.iter().chain(&b).all(|w| (0.2..=0.8).contains(w)),
        "{a:?} {b:?}"
    );
    assert_ne!(a, b, "the weights move over time");
    assert!(
        a.windows(2).any(|p| (p[0] - p[1]).abs() > 1e-3),
        "letters apart: {a:?}"
    );
    s.correlation = Property::fixed(100.0);
    let together = s.wiggly_weights(6, 0.3);
    assert!(
        together.windows(2).all(|p| (p[0] - p[1]).abs() < 1e-6),
        "{together:?}"
    );
    assert_eq!(s.wiggly_weights(6, 0.3), together);
}

#[test]
fn an_animators_property_list_reads_back() {
    let mut a = TextAnimator::new("A");
    a.properties = Some(vec!["scale".into(), "position".into()]);
    let back: TextAnimator = serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
    assert_eq!(back.properties, a.properties);
    let plain = serde_json::to_value(TextAnimator::new("B")).unwrap();
    assert!(plain.get("properties").is_none());
}

#[test]
fn every_wireable_number_is_found_by_its_port() {
    for kind in [
        SelectorKind::Range,
        SelectorKind::Proximity,
        SelectorKind::Wiggly,
        SelectorKind::Stagger,
    ] {
        let mut a = TextAnimator::new("A");
        a.selector.kind = kind;
        a.axes = vec![AnimatorAxis {
            tag: "wght".into(),
            value: Property::fixed(700.0),
        }];
        let names = a.wireable_numbers();
        assert!(names.iter().any(|(n, _)| n == "axis.wght"), "{kind:?}");
        for (name, label) in &names {
            assert!(!label.is_empty(), "{name}");
            assert!(a.number_mut(name).is_some(), "{kind:?} {name}");
            let port = ports::animator_port(2, name);
            assert_eq!(ports::parse_animator_port(&port), Some((2, name.as_str())));
        }
        assert!(a.number_mut("nothing").is_none());
    }
}

/// Amount keyed from 100 at 0 s to 0 at 1 s, linear.
fn falling_amount() -> Property {
    use crate::anim::{Animation, Keyframe, SideInterp};
    use crate::time::Rational;
    let key = |s: i64, value: f64| Keyframe {
        time: Rational::new(s, 1).unwrap(),
        value,
        interp_in: SideInterp::Linear,
        interp_out: SideInterp::Linear,
    };
    Property {
        animation: Animation::Keyframed(vec![key(0, 100.0), key(1, 0.0)]),
        extra: serde_json::Map::new(),
    }
}

/// **A Stagger selector plays Amount on every unit, one after another**: the
/// first unit is where Amount is, each next one a Delay behind it.
#[test]
fn a_stagger_selector_plays_the_amount_one_unit_after_another() {
    use super::selector::{RangeSelector, SelectorKind};
    let s = RangeSelector {
        kind: SelectorKind::Stagger,
        amount: falling_amount(),
        delay: Property::fixed(0.25),
        ..RangeSelector::default()
    };
    let w = s.stagger_weights(3, 0.5);
    // Units 0, 1, 2 read Amount at 0.5, 0.25 and 0 s.
    for (got, want) in w.iter().zip([0.5f32, 0.75, 1.0]) {
        assert!((got - want).abs() < 1e-6, "{w:?}");
    }
    // Subtract turns them over.
    let sub = RangeSelector {
        invert: true,
        ..s.clone()
    }
    .stagger_weights(3, 0.5);
    for (a, b) in w.iter().zip(&sub) {
        assert!((a + b - 1.0).abs() < 1e-6, "{w:?} {sub:?}");
    }
}

/// The orders: Backward starts the last unit first, From centre the middle,
/// From edges both ends, Random an order of its own for each seed.
#[test]
fn a_stagger_selector_starts_its_units_in_order() {
    use super::selector::{RangeSelector, SelectorKind, StaggerOrder};
    let at = |order: StaggerOrder, seed: u32| {
        RangeSelector {
            kind: SelectorKind::Stagger,
            amount: falling_amount(),
            delay: Property::fixed(0.1),
            order,
            seed,
            ..RangeSelector::default()
        }
        .stagger_weights(5, 0.3)
    };
    // A unit further on in the order has fallen less far.
    let back = at(StaggerOrder::Backward, 0);
    assert!(back[4] < back[0], "{back:?}");
    let centre = at(StaggerOrder::FromCentre, 0);
    assert!(centre[2] < centre[1] && centre[1] < centre[0], "{centre:?}");
    assert!((centre[1] - centre[3]).abs() < 1e-6, "{centre:?}");
    let edges = at(StaggerOrder::FromEdges, 0);
    assert!(edges[0] < edges[1] && edges[1] < edges[2], "{edges:?}");
    let (a, b) = (at(StaggerOrder::Random, 1), at(StaggerOrder::Random, 2));
    assert_ne!(a, b);
    assert_eq!(a, at(StaggerOrder::Random, 1));
}

/// Delay ease shares the delays out unevenly: above 0 the first units start
/// close together, so the second has fallen nearly as far as the first.
#[test]
fn delay_ease_bunches_the_first_starts() {
    use super::selector::{RangeSelector, SelectorKind};
    let mut s = RangeSelector {
        kind: SelectorKind::Stagger,
        amount: falling_amount(),
        delay: Property::fixed(0.2),
        ..RangeSelector::default()
    };
    let even = s.stagger_weights(4, 0.7);
    s.delay_ease = Property::fixed(100.0);
    let eased = s.stagger_weights(4, 0.7);
    assert!(eased[1] < even[1], "{even:?} {eased:?}");
    // The last still starts the whole spread after the first.
    assert!((eased[3] - even[3]).abs() < 1e-6, "{even:?} {eased:?}");
}

/// **A Stagger's bounce** swings each unit on past Amount's last key by the
/// speed it arrived with, as the inertial-bounce expression does, and dies
/// away; an amplitude of 0 leaves Amount as it is.
#[test]
fn a_stagger_bounces_past_its_last_key_and_settles() {
    use super::selector::{RangeSelector, SelectorKind};
    let mut s = RangeSelector {
        kind: SelectorKind::Stagger,
        amount: falling_amount(),
        delay: Property::fixed(0.0),
        ..RangeSelector::default()
    };
    assert_eq!(s.stagger_weights(1, 1.125), vec![0.0]);
    s.bounce_amplitude = Property::fixed(3.0);
    // Falling at 100 % a second, it swings on below 0 a quarter swing after
    // the key, back above it at three quarters, and is still by 3 s.
    let w = |t: f64| s.stagger_weights(1, t)[0];
    assert!(w(1.125) < -0.015, "{}", w(1.125));
    assert!(w(1.375) > 0.005, "{}", w(1.375));
    assert!(w(3.0).abs() < 1e-3, "{}", w(3.0));
    // Before the last key nothing is added.
    assert!((w(0.5) - 0.5).abs() < 1e-6, "{}", w(0.5));
}

// ---- Engine changes 104 on: more selectors, the eye, Amount, Follow -------

/// A Range selector over the whole run, square, with its window.
fn window(start: f64, end: f64) -> super::selector::RangeSelector {
    super::selector::RangeSelector {
        start: Property::fixed(start),
        end: Property::fixed(end),
        ..super::selector::RangeSelector::default()
    }
}

/// An animator that fades its letters out entirely.
fn fader(name: &str) -> TextAnimator {
    TextAnimator {
        opacity: Property::fixed(0.0),
        ..TextAnimator::new(name)
    }
}

fn opacities(animators: &[TextAnimator], text: &str, lt: f64) -> Vec<f32> {
    glyph_xforms(animators, text, lt, &[])
        .iter()
        .map(|x| x.opacity)
        .collect()
}

/// **Selectors put together top to bottom, as After Effects' Modes**: the
/// first selector reaches the first three of six letters, the second the
/// middle three, under each mode.
#[test]
fn a_second_selector_is_put_together_by_its_mode() {
    use super::selector::SelectorCombine;
    let first = window(0.0, 50.0);
    let both = |combine: SelectorCombine, invert: bool| {
        let mut second = window(100.0 / 3.0, 250.0 / 3.0);
        second.combine = combine;
        second.invert = invert;
        let a = TextAnimator {
            selector: first.clone(),
            selectors: vec![second],
            ..fader("A")
        };
        // Faded by the weight: opacity 1 − w.
        opacities(&[a], "abcdef", 0.0)
            .iter()
            .map(|o| 1.0 - o)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        both(SelectorCombine::Add, false),
        [1.0, 1.0, 1.0, 1.0, 1.0, 0.0]
    );
    assert_eq!(
        both(SelectorCombine::Add, true),
        [1.0, 1.0, 0.0, 0.0, 0.0, 0.0]
    );
    assert_eq!(
        both(SelectorCombine::Intersect, false),
        [0.0, 0.0, 1.0, 0.0, 0.0, 0.0]
    );
    assert_eq!(
        both(SelectorCombine::Min, false),
        [0.0, 0.0, 1.0, 0.0, 0.0, 0.0]
    );
    assert_eq!(
        both(SelectorCombine::Max, false),
        [1.0, 1.0, 1.0, 1.0, 1.0, 0.0]
    );
    assert_eq!(
        both(SelectorCombine::Difference, false),
        [1.0, 1.0, 0.0, 1.0, 1.0, 0.0]
    );
}

/// **The eye and solo**: a switched-off animator counts for nothing, and
/// while one is soloed the others do not count.
#[test]
fn the_eye_and_solo_decide_which_animators_count() {
    let mut a = fader("A");
    let mut b = TextAnimator {
        position_y: Property::fixed(10.0),
        ..TextAnimator::new("B")
    };
    a.enabled = false;
    let x = glyph_xforms(&[a.clone(), b.clone()], "ab", 0.0, &[]);
    assert_eq!(x[0].opacity, 1.0);
    assert_eq!(x[0].position[1], 10.0);
    a.enabled = true;
    b.solo = true;
    let x = glyph_xforms(&[a.clone(), b.clone()], "ab", 0.0, &[]);
    assert_eq!(x[0].opacity, 1.0, "a soloed B leaves A out");
    assert_eq!(x[0].position[1], 10.0);
    // A soloed animator that is switched off does not silence the rest.
    b.enabled = false;
    let x = glyph_xforms(&[a, b], "ab", 0.0, &[]);
    assert_eq!(x[0].opacity, 0.0);
    assert_eq!(x[0].position[1], 0.0);
}

/// **An animator's own Amount scales its whole effect.**
#[test]
fn an_animators_amount_scales_every_letter() {
    let a = TextAnimator {
        amount: Property::fixed(25.0),
        position_y: Property::fixed(40.0),
        ..TextAnimator::new("A")
    };
    let x = glyph_xforms(&[a], "ab", 0.0, &[]);
    assert!((x[1].position[1] - 10.0).abs() < 1e-5);
}

/// **Follow**: an animator following another reads that one's selection,
/// the offset later, and a pair following each other reaches nothing.
#[test]
fn an_animator_follows_another_ones_selection() {
    use super::selector::{RangeSelector, SelectorKind};
    let lead = TextAnimator {
        selector: RangeSelector {
            kind: SelectorKind::Stagger,
            amount: falling_amount(),
            delay: Property::fixed(0.25),
            ..RangeSelector::default()
        },
        position_y: Property::fixed(100.0),
        ..TextAnimator::new("Drop")
    };
    let fade = TextAnimator {
        follow: Some("Drop".into()),
        follow_offset: 0.5,
        ..fader("Fade")
    };
    // At 0.75 s the follower reads the lead's selection at 0.25 s.
    let both = glyph_xforms(&[lead.clone(), fade.clone()], "ab", 0.75, &[]);
    let lead_then = glyph_xforms(std::slice::from_ref(&lead), "ab", 0.25, &[]);
    for (x, then) in both.iter().zip(&lead_then) {
        let w = then.position[1] / 100.0;
        assert!((x.opacity - (1.0 - w)).abs() < 1e-5, "{x:?} {then:?}");
    }
    // Following each other: neither reaches a letter.
    let a = TextAnimator {
        follow: Some("B".into()),
        ..fader("A")
    };
    let b = TextAnimator {
        follow: Some("A".into()),
        ..fader("B")
    };
    assert!(opacities(&[a, b], "ab", 0.0).iter().all(|o| *o == 1.0));
}

/// **Characters excluding spaces and lines**: a space is not a unit of its
/// own, and a line break ends a line.
#[test]
fn spaces_are_left_out_and_lines_are_counted() {
    let (units, n) = unit_indices("ab c", SelectorBasis::CharactersExcludingSpaces);
    assert_eq!((units, n), (vec![0, 1, 1, 2], 3));
    let (units, n) = unit_indices("ab\ncd\ne", SelectorBasis::Lines);
    assert_eq!((units, n), (vec![0, 0, 0, 1, 1, 1, 2], 3));
    assert_eq!(unit_indices("", SelectorBasis::Lines), (vec![], 0));
}

/// **Index units** count letters: 1 to 3 reaches the second and third.
#[test]
fn index_units_count_letters() {
    let mut s = window(1.0, 3.0);
    s.units = super::selector::SelectorUnits::Index;
    assert_eq!(s.weights_at(5, 0.0), [0.0, 1.0, 1.0, 0.0, 0.0]);
}

/// **Randomize order** moves the same window over the letters in another
/// order: as many letters reached, not the same ones, the same for a seed.
#[test]
fn randomize_order_reaches_the_same_number_in_another_order() {
    let mut s = window(0.0, 50.0);
    let plain = s.weights_at(10, 0.0);
    s.randomize = true;
    s.seed = 3;
    let shuffled = s.weights_at(10, 0.0);
    assert_eq!(shuffled.iter().sum::<f32>(), plain.iter().sum::<f32>());
    assert_ne!(shuffled, plain);
    assert_eq!(shuffled, s.weights_at(10, 0.0));
}

/// **Total duration**: the spread stays the Delay however many units.
#[test]
fn total_duration_keeps_the_spread_whatever_the_length() {
    use super::selector::{RangeSelector, SelectorKind};
    let s = RangeSelector {
        kind: SelectorKind::Stagger,
        amount: falling_amount(),
        delay: Property::fixed(0.5),
        total: true,
        ..RangeSelector::default()
    };
    // The last unit starts 0.5 s after the first, for 3 units or 11.
    for units in [3, 11] {
        let w = s.stagger_weights(units, 0.5);
        assert!((w[units - 1] - 1.0).abs() < 1e-6);
        assert!((w[0] - 0.5).abs() < 1e-6);
    }
}

/// **Loop and ping-pong**: after Amount's last key a Loop starts again from
/// the first, a Ping-pong plays back.
#[test]
fn a_stagger_loops_and_ping_pongs_after_the_last_key() {
    use super::selector::{RangeSelector, SelectorKind, StaggerRepeat};
    let mut s = RangeSelector {
        kind: SelectorKind::Stagger,
        amount: falling_amount(),
        ..RangeSelector::default()
    };
    assert!((s.repeated(1.25) - 1.25).abs() < 1e-9);
    s.repeat = StaggerRepeat::Loop;
    assert!((s.repeated(1.25) - 0.25).abs() < 1e-9);
    assert!((s.repeated(2.5) - 0.5).abs() < 1e-9);
    s.repeat = StaggerRepeat::PingPong;
    assert!((s.repeated(1.25) - 0.75).abs() < 1e-9);
    assert!((s.repeated(2.25) - 0.25).abs() < 1e-9);
    assert!(
        (s.repeated(0.5) - 0.5).abs() < 1e-9,
        "before the last key, as it is"
    );
}

/// **Bounce on the last key only** leaves the keys before it unbounced.
#[test]
fn bounce_on_the_last_key_only() {
    use super::selector::{RangeSelector, SelectorKind};
    use crate::anim::{Animation, Keyframe, SideInterp};
    use crate::time::Rational;
    let key = |s: i64, value: f64| Keyframe {
        time: Rational::new(s, 1).unwrap(),
        value,
        interp_in: SideInterp::Linear,
        interp_out: SideInterp::Linear,
    };
    let mut s = RangeSelector {
        kind: SelectorKind::Stagger,
        amount: Property {
            animation: Animation::Keyframed(vec![
                key(0, 0.0),
                key(1, 100.0),
                key(2, 50.0),
                key(3, 0.0),
            ]),
            extra: serde_json::Map::new(),
        },
        bounce_amplitude: Property::fixed(20.0),
        ..RangeSelector::default()
    };
    // Just after the second key the bounce is there, then not.
    let at = |s: &RangeSelector| s.stagger_weights(1, 1.1)[0];
    let bounced = at(&s);
    s.bounce_last = true;
    let plain = at(&s);
    assert!((bounced - plain).abs() > 0.01);
    assert!((plain - 0.95).abs() < 1e-6);
}

/// **Skew, tracking, blur and character offset** are carried to the letters
/// by the weight, and none of them is there at their defaults.
#[test]
fn the_new_properties_reach_the_letters_by_weight() {
    let a = TextAnimator {
        selector: window(0.0, 50.0),
        skew: Property::fixed(20.0),
        skew_axis: Property::fixed(30.0),
        tracking: Property::fixed(100.0),
        blur: Property::fixed(4.0),
        character_offset: Property::fixed(1.0),
        ..TextAnimator::new("A")
    };
    let x = glyph_xforms(std::slice::from_ref(&a), "ab", 0.0, &[]);
    assert_eq!(
        (
            x[0].skew,
            x[0].skew_axis,
            x[0].tracking,
            x[0].blur,
            x[0].char_offset
        ),
        (20.0, 30.0, 100.0, 4.0, 1.0)
    );
    assert!(x[1].is_identity());
    assert_eq!(offset_text("ab", &x).as_deref(), Some("bb"));
    assert!(glyph_xforms(&[TextAnimator::new("B")], "ab", 0.0, &[])
        .iter()
        .all(GlyphXform::is_identity));
}

/// **Character offset** goes round its own run, and leaves the rest.
#[test]
fn character_offset_goes_round_its_run() {
    assert_eq!(offset_char('z', 1), 'a');
    assert_eq!(offset_char('A', -1), 'Z');
    assert_eq!(offset_char('9', 2), '1');
    assert_eq!(offset_char(' ', 5), ' ');
    assert_eq!(offset_char('!', 5), '!');
}

/// **Old files read the same, and new rows at their defaults write nothing.**
#[test]
fn an_animator_at_its_defaults_writes_none_of_the_new_rows() {
    let a = TextAnimator::new("A");
    assert_eq!(serde_json::to_string(&a).unwrap(), "{\"name\":\"A\"}");
    let back: TextAnimator = serde_json::from_str("{\"name\":\"A\"}").unwrap();
    assert_eq!(back, a);
    assert!(back.enabled);
}

#[test]
fn a_second_selector_has_ports_of_its_own() {
    let mut a = TextAnimator::new("A");
    let mut second = a.selector.clone();
    second.kind = SelectorKind::Stagger;
    a.selectors.push(second);
    let names = a.wireable_numbers();
    assert!(names
        .iter()
        .any(|(n, l)| n == "start" && l == "Range start"));
    assert!(names
        .iter()
        .any(|(n, l)| n == "s1.delay" && l == "Selector 2 · Delay"));
    for (name, _) in &names {
        assert!(a.number_mut(name).is_some(), "{name}");
    }
    // The second is a Stagger: no Range numbers, and no third selector.
    assert!(a.number_mut("s1.start").is_none());
    assert!(a.number_mut("s2.amount").is_none());
    // A property starting with "s" is not taken for a selector.
    assert!(a.number_mut("scale_x").is_some());
    let port = ports::animator_port(0, "s1.delay");
    assert_eq!(ports::parse_animator_port(&port), Some((0, "s1.delay")));
}

/// **A wire follows its animator.** Moving, renaming or taking out an
/// animator (or one of its selectors) carries the Node graph's wires into
/// its numbers along, and undo puts them back exactly.
#[test]
fn the_wires_follow_the_animators_through_an_edit_and_its_undo() {
    use crate::edits::{base_layer, new_comp_ops, CompSettings};
    use crate::graph::{Edge, InputRef, NodeRef, OutputRef};
    use crate::model::{Document, LayerKind, LinearColour, TextDocument};
    use crate::ops::Op;

    let wire = |port: String| Edge {
        from: OutputRef::Driver {
            node: uuid::Uuid::nil(),
            port: "value".into(),
        },
        to: InputRef::Param {
            node: NodeRef::Source,
            port,
        },
    };
    let ports = |doc: &Document, comp, layer| -> Vec<String> {
        let l = doc
            .comp(comp)
            .unwrap()
            .layers
            .iter()
            .find(|l| l.id == layer)
            .unwrap();
        l.graph
            .edges
            .iter()
            .map(|e| match &e.to {
                InputRef::Param { port, .. } => port.clone(),
                InputRef::Matte { .. } => String::new(),
            })
            .collect()
    };

    let mut doc = Document::new();
    let (comp, ops) = new_comp_ops(&doc, CompSettings::default(), None);
    for op in &ops {
        crate::ops::apply(&mut doc, op).unwrap();
    }
    let duration = doc.comp(comp).unwrap().duration.0;
    let mut b = TextAnimator::new("B");
    b.selectors.push(b.selector.clone());
    b.selectors[0].kind = SelectorKind::Stagger;
    let document = TextDocument {
        text: "Lumit".into(),
        expression: None,
        size: 60.0,
        fill: LinearColour([1.0, 1.0, 1.0, 1.0]),
        path: None,
        path_offset: Property::zero(),
        animators: vec![TextAnimator::new("A"), b],
        style: Default::default(),
        paragraph: Default::default(),
        captions: None,
        keys: None,
        extra: serde_json::Map::new(),
    };
    let mut layer = base_layer(
        "Words".into(),
        LayerKind::Text {
            document: document.clone(),
        },
        duration,
        Default::default(),
    );
    layer.graph.edges = vec![
        wire(ports::animator_port(0, "offset")),
        wire(ports::animator_port(1, "start")),
        wire(ports::animator_port(1, "s1.delay")),
        wire(ports::animator_port(1, "opacity")),
    ];
    let id = layer.id;
    crate::ops::apply(
        &mut doc,
        &Op::AddLayer {
            comp,
            index: 0,
            layer: Box::new(layer),
        },
    )
    .unwrap();
    let set = |doc: &mut Document, animators: Vec<TextAnimator>| {
        let mut next = document.clone();
        next.animators = animators;
        crate::ops::apply(
            doc,
            &Op::SetTextDocument {
                comp,
                layer: id,
                document: next,
                graph: None,
            },
        )
        .unwrap()
    };
    let [a, b] = [document.animators[0].clone(), document.animators[1].clone()];

    // B moved above A: every wire follows.
    let undo = set(&mut doc, vec![b.clone(), a.clone()]);
    assert_eq!(
        ports(&doc, comp, id),
        [
            "text.1.offset",
            "text.0.start",
            "text.0.s1.delay",
            "text.0.opacity"
        ]
    );
    crate::ops::apply(&mut doc, &undo).unwrap();
    assert_eq!(
        ports(&doc, comp, id),
        [
            "text.0.offset",
            "text.1.start",
            "text.1.s1.delay",
            "text.1.opacity"
        ]
    );

    // A taken out: its wire goes, B's follow it up one place.
    let undo = set(&mut doc, vec![b.clone()]);
    assert_eq!(
        ports(&doc, comp, id),
        ["text.0.start", "text.0.s1.delay", "text.0.opacity"]
    );
    crate::ops::apply(&mut doc, &undo).unwrap();
    assert_eq!(ports(&doc, comp, id).len(), 4, "undo brings the wire back");

    // B renamed: its wires stay on it.
    let mut renamed = b.clone();
    renamed.name = "Wave".into();
    set(&mut doc, vec![a.clone(), renamed.clone()]);
    assert_eq!(
        ports(&doc, comp, id),
        [
            "text.0.offset",
            "text.1.start",
            "text.1.s1.delay",
            "text.1.opacity"
        ]
    );

    // Its first selector taken out: the second's wire becomes the first's.
    let mut fewer = renamed.clone();
    fewer.selector = fewer.selectors.remove(0);
    set(&mut doc, vec![a, fewer]);
    assert_eq!(
        ports(&doc, comp, id),
        ["text.0.offset", "text.1.delay", "text.1.opacity"]
    );
}

/// A selector turned into another kind takes off the wires into numbers it
/// no longer has, and keeps the rest.
#[test]
fn a_selector_of_another_kind_drops_the_wires_it_cannot_take() {
    use crate::graph::{Edge, InputRef, LayerGraph, NodeRef, OutputRef};
    let wire = |port: &str| Edge {
        from: OutputRef::Driver {
            node: uuid::Uuid::nil(),
            port: "value".into(),
        },
        to: InputRef::Param {
            node: NodeRef::Source,
            port: ports::animator_port(0, port),
        },
    };
    let mut graph = LayerGraph {
        edges: vec![wire("start"), wire("opacity")],
        ..LayerGraph::default()
    };
    let old = vec![TextAnimator::new("A")];
    let mut new = old.clone();
    new[0].selector.kind = SelectorKind::Stagger;
    assert!(ports::remap_text_ports(&mut graph, &old, &new));
    assert_eq!(graph.edges, vec![wire("opacity")]);
}

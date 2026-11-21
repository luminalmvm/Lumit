//! Tests for text animators.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::selector::{near_weight, SelectorShape};
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
    assert!(steep_top.iter().all(|w| (0.0..=1.0).contains(w)), "{steep_top:?}");
    // Past the range the letters are moved the whole way, eased or not.
    s.offset = Property::fixed(-100.0);
    assert!(s.weights_at(4, 0.0).iter().all(|w| (*w - 1.0).abs() < 1e-6));
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

/// **A Proximity selector moves the letters near its target**: all the
/// way on it, none from the radius out, and the shape in between.
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

    // Smooth eases the slope: half way is still half, a quarter in is less.
    let smooth = near_weight(SelectorShape::Smooth, 25.0, 100.0);
    assert!((near_weight(SelectorShape::Smooth, 50.0, 100.0) - 0.5).abs() < 1e-6);
    assert!(smooth > 0.75 && smooth < 1.0, "{smooth}");

    // Inverted, the far letters move and the near ones stay.
    let mut inverted = proximity(100.0, SelectorShape::Ramp, target);
    inverted.selector.invert = true;
    let x = glyph_xforms_near(&[inverted], "abc", 0.0, &[], &near);
    assert_eq!(x[0].position[1], 0.0);
    assert_eq!(x[2].position[1], -10.0);
}

/// A Proximity selector whose target isn't there reaches nothing, the same
/// as an empty range.
#[test]
fn a_proximity_selector_without_its_target_reaches_nothing() {
    let animator = proximity(100.0, SelectorShape::Ramp, uuid::Uuid::from_u128(1));
    let x = glyph_xforms(&[animator], "ab", 0.0, &[]);
    assert!(x.iter().all(GlyphXform::is_identity), "{x:?}");
}

/// **A font axis is a place, mixed by the weight**: a letter half reached
/// is set half way from the layer's place to the animator's, and a second
/// animator carries on from where the first left it.
#[test]
fn a_font_axis_is_set_half_way_by_half_a_weight() {
    let mut a = TextAnimator::new("Weight");
    a.selector.shape = SelectorShape::Ramp;
    a.axes.push(AnimatorAxis {
        tag: "wght".into(),
        value: Property::fixed(900.0),
    });
    // Two letters: the ramp gives the first a quarter, the second three.
    let x = glyph_xforms(&[a.clone()], "ab", 0.0, &[]);
    let base = |_: [u8; 4]| 100.0f32;
    let at = |x: &GlyphXform| x.axes.resolved(&base).next().unwrap();
    assert_eq!(at(&x[0]).0, *b"wght");
    assert!((at(&x[0]).1 - 300.0).abs() < 1e-3, "{:?}", at(&x[0]));
    assert!((at(&x[1]).1 - 700.0).abs() < 1e-3, "{:?}", at(&x[1]));

    // A second animator taking the whole word to 500 wins over the first.
    let mut b = TextAnimator::new("Back");
    b.axes.push(AnimatorAxis {
        tag: "wght".into(),
        value: Property::fixed(500.0),
    });
    let x = glyph_xforms(&[a, b], "ab", 0.0, &[]);
    assert!((at(&x[0]).1 - 500.0).abs() < 1e-3);
    assert_eq!(x[0].axes.len, 1);
}

/// No more than eight axes, and a tag that isn't one is left out.
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

/// The new settings write nothing while they hold their defaults, and
/// read back what was set.
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

/// **A Wiggly selector wanders**: every weight stays between the two
/// amounts, changes over time, and at full correlation the letters move
/// as one.
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
    // The same settings wander the same way every time.
    assert_eq!(s.wiggly_weights(6, 0.3), together);
}

/// An animator's list of property groups is written only once there is
/// one, and reads back.
#[test]
fn an_animators_property_list_reads_back() {
    let mut a = TextAnimator::new("A");
    a.properties = Some(vec!["scale".into(), "position".into()]);
    let back: TextAnimator = serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
    assert_eq!(back.properties, a.properties);
    let plain = serde_json::to_value(TextAnimator::new("B")).unwrap();
    assert!(plain.get("properties").is_none());
}

/// **Every socket on the Source box reaches its number**: each name an
/// animator lists for wiring finds the number it names and comes back from
/// its port, or a wire into it would move nothing.
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
    let sub = RangeSelector { invert: true, ..s.clone() }.stagger_weights(3, 0.5);
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

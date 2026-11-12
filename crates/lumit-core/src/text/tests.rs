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
    let x = glyph_xforms(&[a], "ab cd", 0.0);
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
    let x = glyph_xforms(&[drop, fade], "A", 0.0);
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
    let x = glyph_xforms(&[a], "ab", 0.0);
    // Two letters: their middles sit at 25 % and 75 % of the run.
    assert!((x[0].position[0] - 25.0).abs() < 1e-4, "{x:?}");
    assert!((x[1].position[0] - 75.0).abs() < 1e-4, "{x:?}");
    assert!((x[0].scale[0] - 1.25).abs() < 1e-6, "{x:?}");
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
        &near,
    );
    let pushes: Vec<f32> = ramp.iter().map(|x| x.position[1]).collect();
    assert_eq!(pushes, vec![-10.0, -5.0, 0.0, 0.0]);

    let square = glyph_xforms_near(
        &[proximity(60.0, SelectorShape::Square, target)],
        "abc\n",
        0.0,
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
    let x = glyph_xforms_near(&[inverted], "abc", 0.0, &near);
    assert_eq!(x[0].position[1], 0.0);
    assert_eq!(x[2].position[1], -10.0);
}

/// A Proximity selector whose target isn't there reaches nothing, the same
/// as an empty range.
#[test]
fn a_proximity_selector_without_its_target_reaches_nothing() {
    let animator = proximity(100.0, SelectorShape::Ramp, uuid::Uuid::from_u128(1));
    let x = glyph_xforms(&[animator], "ab", 0.0);
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
    let x = glyph_xforms(&[a.clone()], "ab", 0.0);
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
    let x = glyph_xforms(&[a, b], "ab", 0.0);
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
    let x = glyph_xforms(&[a], "a", 0.0);
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


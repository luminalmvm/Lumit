//! Timeline lane-span tests for the shell (moved verbatim from mod.rs).

use super::*;
use lumit_core::model::LayerKind;
use lumit_core::retime::Retime;
use lumit_core::Rational;

fn rat(n: i64, d: i64) -> Rational {
    Rational::new(n, d).unwrap()
}

// 2× over a 4 s layer eats 4 s of source by local 2 s — with the layer
// starting at comp 1 s, the held tail runs from comp 3 s to the out point.
#[test]
fn overrun_span_runs_from_exhaustion_to_the_out_point() {
    let rt = Retime::constant_speed(rat(4, 1), rat(0, 1), rat(2, 1));
    let (start, end) = overrun_span_secs(&rt, 4.0, 1.0, 1.0, 5.0).unwrap();
    assert!((start - 3.0).abs() < 1e-3, "held tail starts at {start}");
    assert!((end - 5.0).abs() < 1e-9, "held tail ends at {end}");
}

// Plenty of source at 1× — nothing to indicate.
#[test]
fn no_overrun_means_no_span() {
    let rt = Retime::constant_speed(rat(4, 1), rat(0, 1), rat(1, 1));
    assert_eq!(overrun_span_secs(&rt, 10.0, 0.0, 0.0, 4.0), None);
}

// Source-in already past the media's end: the whole visible bar is a
// hold, and the span clamps to start at the in point (the bar's left
// edge), not at layer time zero.
#[test]
fn overrun_before_the_in_point_clamps_to_the_bar() {
    let rt = Retime::identity(rat(4, 1), rat(5, 1));
    let (start, end) = overrun_span_secs(&rt, 3.0, 0.0, 2.0, 4.0).unwrap();
    assert!(
        (start - 2.0).abs() < 1e-9,
        "span clamps to in point, got {start}"
    );
    assert!(
        (end - 4.0).abs() < 1e-9,
        "span ends at out point, got {end}"
    );
}

// The source runs out only past the layer's out point — the overrun is
// real in the map but invisible on the bar, so no span.
#[test]
fn overrun_past_the_out_point_is_not_drawn() {
    let rt = Retime::constant_speed(rat(4, 1), rat(0, 1), rat(2, 1));
    assert_eq!(overrun_span_secs(&rt, 4.0, 0.0, 0.0, 1.5), None);
}

// §6.1: every layer kind paints its own identity colour on the lane bar
// (the adjustment layer borrows the solid's until it earns its own).
#[test]
fn every_layer_kind_paints_its_identity_colour() {
    let theme = Theme::dark();
    let id = uuid::Uuid::now_v7();
    let text = lumit_core::model::TextDocument {
        text: String::new(),
        size: 12.0,
        fill: lumit_core::model::LinearColour::BLACK,
        extra: serde_json::Map::new(),
    };
    let cases = [
        (
            LayerKind::Footage {
                item: id,
                retime: None,
            },
            theme.layer.footage,
        ),
        (
            LayerKind::Sequence { clips: Vec::new() },
            theme.layer.sequence,
        ),
        (LayerKind::Precomp { comp: id }, theme.layer.precomp),
        (LayerKind::Solid { def: id }, theme.layer.solid),
        (LayerKind::Text { document: text }, theme.layer.text),
        (
            LayerKind::Camera {
                zoom: lumit_core::anim::Property::fixed(1000.0),
            },
            theme.layer.camera,
        ),
        (LayerKind::Adjustment, theme.layer.solid),
    ];
    for (kind, want) in cases {
        assert_eq!(layer_type_style(&kind, &theme).1, want);
    }
}

// The identity family must stay distinct siblings — two types sharing a
// colour would make the timeline lie about what a bar is.
#[test]
fn identity_colours_are_distinct() {
    let t = Theme::dark();
    let all = [
        t.layer.footage,
        t.layer.sequence,
        t.layer.precomp,
        t.layer.solid,
        t.layer.text,
        t.layer.camera,
    ];
    for (i, a) in all.iter().enumerate() {
        for b in &all[i + 1..] {
            assert_ne!(a, b, "identity colours collide");
        }
    }
}

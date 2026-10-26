use super::modifier::{self, ModifierKind, Offset, Repeater, ShapeEntry, ShapeModifier, Trim};
use super::raster::dashed;
use super::*;
use crate::mask::{Mask, Vertex};

pub(super) fn square(x: f64, y: f64, side: f64) -> BezierPath {
    Mask::rectangle(x, y, side, side).path
}

pub(super) fn item(path: BezierPath) -> ShapeItem {
    ShapeItem::filled("Rectangle", path, LinearColour([1.0, 0.0, 0.0, 1.0]))
}

/// `items` as a layer's contents, with nothing acting on them.
pub(super) fn entries(items: Vec<ShapeItem>) -> Vec<ShapeEntry> {
    items.into_iter().map(ShapeEntry::Item).collect()
}

/// A modifier entry of `kind`, named for it.
pub(super) fn modifier(kind: ModifierKind) -> ShapeEntry {
    ShapeEntry::Modifier(ShapeModifier {
        id: Uuid::now_v7(),
        name: kind.label().into(),
        kind,
    })
}

/// One item with `kinds` acting on it, in order.
pub(super) fn with(it: ShapeItem, kinds: Vec<ModifierKind>) -> Vec<ShapeEntry> {
    let mut out = entries(vec![it]);
    out.extend(kinds.into_iter().map(modifier));
    out
}

/// The first item's art at `t` once its modifiers have had it, as the
/// rasteriser sees it: `None` where nothing moved it.
pub(super) fn points_of(contents: &[ShapeEntry], t: f64) -> Option<Vec<(f64, f64)>> {
    let art = modifier::lowered(contents);
    let first = art.first()?;
    first.item.trimmed_at(&first.mods, t)
}

fn repeater(copies: f64, x: f64) -> Repeater {
    Repeater {
        copies: Property::fixed(copies),
        offset: Property::zero(),
        anchor_x: Property::zero(),
        anchor_y: Property::zero(),
        position_x: Property::fixed(x),
        position_y: Property::zero(),
        rotation: Property::zero(),
        scale: crate::paint::full(),
        start_opacity: crate::paint::full(),
        end_opacity: crate::paint::full(),
        extra: serde_json::Map::new(),
    }
}

fn trim(start: f64, end: f64) -> ModifierKind {
    ModifierKind::Trim(Trim {
        start: Property::fixed(start),
        end: Property::fixed(end),
        offset: Property::zero(),
        extra: serde_json::Map::new(),
    })
}

fn offset(amount: f64) -> ModifierKind {
    ModifierKind::Offset(Offset {
        amount: Property::fixed(amount),
        extra: serde_json::Map::new(),
    })
}

pub(super) fn alpha_at(rgba: &[u8], w: u32, x: u32, y: u32) -> u8 {
    rgba[((y * w + x) as usize) * 4 + 3]
}

fn rgb_at(rgba: &[u8], w: u32, x: u32, y: u32) -> [u8; 3] {
    let i = ((y * w + x) as usize) * 4;
    [rgba[i], rgba[i + 1], rgba[i + 2]]
}

#[test]
fn a_layers_size_is_the_box_its_art_fills() {
    let contents = entries(vec![item(square(10.0, 20.0, 30.0))]);
    assert_eq!(
        contents_bounds(&contents, 0.0),
        Some((10.0, 20.0, 40.0, 50.0))
    );

    // Two items: the box holds both.
    let contents = entries(vec![
        item(square(10.0, 20.0, 30.0)),
        item(square(-5.0, 0.0, 8.0)),
    ]);
    assert_eq!(
        contents_bounds(&contents, 0.0),
        Some((-5.0, 0.0, 40.0, 50.0))
    );

    assert!(contents_bounds(&[], 0.0).is_none());
}

#[test]
fn a_filled_shape_draws_its_colour_inside_and_nothing_outside() {
    let contents = entries(vec![item(square(0.0, 0.0, 20.0))]);
    // Rasterised into its own bounding box at 1:1.
    let rgba = rasterise_contents(&contents, 20, 20, 0.0, 0.0, 20.0, 20.0, 0.0);
    assert_eq!(alpha_at(&rgba, 20, 10, 10), 255, "inside the square");
    assert_eq!(rgb_at(&rgba, 20, 10, 10), [255, 0, 0]);

    // A smaller square in a bigger box leaves the rest transparent.
    let contents = entries(vec![item(square(5.0, 5.0, 5.0))]);
    let rgba = rasterise_contents(&contents, 20, 20, 0.0, 0.0, 20.0, 20.0, 0.0);
    assert_eq!(alpha_at(&rgba, 20, 7, 7), 255);
    assert_eq!(alpha_at(&rgba, 20, 18, 18), 0, "outside the art");
}

#[test]
fn a_fill_and_a_stroke_are_both_drawn_and_the_stroke_is_on_top() {
    let mut it = item(square(4.0, 4.0, 12.0));
    it.stroke = Some(colour::still(LinearColour([0.0, 1.0, 0.0, 1.0])));
    it.stroke_width = Property::fixed(3.0);
    let rgba = rasterise_contents(&entries(vec![it]), 20, 20, 0.0, 0.0, 20.0, 20.0, 0.0);

    assert_eq!(rgb_at(&rgba, 20, 10, 10), [255, 0, 0], "the fill inside");
    assert_eq!(
        rgb_at(&rgba, 20, 4, 10),
        [0, 255, 0],
        "the stroke on the edge"
    );
}

#[test]
fn items_are_drawn_in_order() {
    let under = item(square(0.0, 0.0, 20.0));
    let mut over = item(square(0.0, 0.0, 20.0));
    over.fill = Some(colour::still(LinearColour([0.0, 0.0, 1.0, 1.0])));
    let rgba = rasterise_contents(
        &entries(vec![under, over]),
        20,
        20,
        0.0,
        0.0,
        20.0,
        20.0,
        0.0,
    );
    assert_eq!(
        rgb_at(&rgba, 20, 10, 10),
        [0, 0, 255],
        "the later item wins"
    );
}

#[test]
fn nothing_in_it_draws_nothing_and_never_panics() {
    assert!(rasterise_contents(&[], 4, 4, 0.0, 0.0, 4.0, 4.0, 0.0)
        .iter()
        .all(|&b| b == 0));
    assert!(rasterise_contents(&[], 0, 0, 0.0, 0.0, 1.0, 1.0, 0.0).is_empty());

    // A path with no vertices, and one with a single vertex.
    let empty = item(BezierPath {
        vertices: Vec::new(),
        closed: true,
    });
    assert!(empty.bounds(&[], 0.0).is_none());
    assert!(
        rasterise_contents(&entries(vec![empty]), 4, 4, 0.0, 0.0, 4.0, 4.0, 0.0)
            .iter()
            .all(|&b| b == 0)
    );
}

/// Total length of a polyline, for the trim tests below.
fn length(points: &[(f64, f64)]) -> f64 {
    points
        .windows(2)
        .map(|p| ((p[1].0 - p[0].0).powi(2) + (p[1].1 - p[0].1).powi(2)).sqrt())
        .sum()
}

fn ink(rgba: &[u8]) -> u32 {
    rgba.chunks_exact(4).map(|p| u32::from(p[3])).sum()
}

#[test]
fn a_trim_cuts_the_path_by_its_own_length() {
    let it = item(square(0.0, 0.0, 20.0));
    let whole = length(&flatten_path(&it.path));
    let start = it.path.vertices[0].pos;
    let half = points_of(&with(it, vec![trim(0.0, 50.0)]), 0.0).expect("a trimmed piece");
    assert!(
        (length(&half) - whole / 2.0).abs() < 1e-6,
        "half the perimeter, got {} of {whole}",
        length(&half)
    );
    // It starts where the path starts.
    assert_eq!(half.first().copied(), Some(start));
}

#[test]
fn a_keyed_trim_is_read_on_the_layers_clock() {
    let key = |secs: i64, value: f64| crate::anim::Keyframe {
        time: crate::time::Rational::new(secs, 1).expect("a whole second"),
        value,
        interp_in: crate::anim::SideInterp::Linear,
        interp_out: crate::anim::SideInterp::Linear,
    };
    let mut end = Property::zero();
    end.animation = crate::anim::Animation::Keyframed(vec![key(0, 0.0), key(1, 100.0)]);
    let contents = with(
        item(square(0.0, 0.0, 20.0)),
        vec![ModifierKind::Trim(Trim {
            start: Property::zero(),
            end,
            offset: Property::zero(),
            extra: serde_json::Map::new(),
        })],
    );
    let at = |t: f64| {
        ink(&rasterise_contents(
            &contents, 20, 20, 0.0, 0.0, 20.0, 20.0, t,
        ))
    };
    assert_eq!(at(0.0), 0, "nothing drawn yet");
    assert!(at(1.0) > at(0.5), "and it fills in as it plays");
}

#[test]
fn paint_from_an_older_file_loads_unchanged_and_a_keyed_fill_moves() {
    let mut it = item(square(0.0, 0.0, 20.0));
    it.stroke = Some(colour::still(LinearColour([0.0, 0.0, 1.0, 1.0])));
    it.stroke_width = Property::fixed(3.0);
    it.opacity = Property::fixed(50.0);
    // A still colour, width and opacity are written as the plain numbers
    // they always were, so a file from before reads back the same.
    let written = serde_json::to_value(&it).expect("writes");
    assert_eq!(written["fill"], serde_json::json!([1.0, 0.0, 0.0, 1.0]));
    assert_eq!(written["stroke_width"], serde_json::json!(3.0));
    assert_eq!(written["opacity"], serde_json::json!(50.0));
    let read: ShapeItem = serde_json::from_value(written).expect("reads");
    assert_eq!(read, it);

    let key = |secs: i64, value: f64| crate::anim::Keyframe {
        time: crate::time::Rational::new(secs, 1).expect("a whole second"),
        value,
        interp_in: crate::anim::SideInterp::Linear,
        interp_out: crate::anim::SideInterp::Linear,
    };
    let mut keyed = item(square(0.0, 0.0, 20.0));
    if let Some(fill) = keyed.fill.as_mut() {
        fill[0].animation = crate::anim::Animation::Keyframed(vec![key(0, 1.0), key(1, 0.0)]);
        fill[2].animation = crate::anim::Animation::Keyframed(vec![key(0, 0.0), key(1, 1.0)]);
    }
    let keyed: ShapeItem =
        serde_json::from_value(serde_json::to_value(&keyed).expect("writes")).expect("reads");
    let contents = entries(vec![keyed]);
    let at = |t: f64| {
        rgb_at(
            &rasterise_contents(&contents, 20, 20, 0.0, 0.0, 20.0, 20.0, t),
            20,
            10,
            10,
        )
    };
    assert_eq!(at(0.0), [255, 0, 0], "red to start");
    assert_eq!(at(1.0), [0, 0, 255], "and blue a second in");
}

#[test]
fn the_dashes_are_cut_by_length_and_the_offset_slides_them() {
    // A straight line 100 long: 10 on, 10 off gives five dashes of ten.
    let line: Vec<(f64, f64)> = vec![(0.0, 0.0), (100.0, 0.0)];
    let cut = |offset: f64| -> Vec<Vec<(f64, f64)>> {
        dashed(&line, &[], &[10.0, 10.0], offset)
            .into_iter()
            .map(|d| d.0)
            .collect()
    };
    let pieces = cut(0.0);
    assert_eq!(pieces.len(), 5);
    assert_eq!(pieces[0], vec![(0.0, 0.0), (10.0, 0.0)]);
    assert_eq!(pieces[1], vec![(20.0, 0.0), (30.0, 0.0)]);

    // An offset of five slides the pattern back along the path, so the
    // first dash is the tail of one that began before the line did.
    let slid = cut(5.0);
    assert_eq!(slid[0], vec![(0.0, 0.0), (5.0, 0.0)]);
    assert_eq!(slid[1], vec![(15.0, 0.0), (25.0, 0.0)]);

    // A whole cycle of offset is no offset at all.
    assert_eq!(cut(20.0), pieces);
}

#[test]
fn a_pattern_too_fine_to_see_is_drawn_solid_rather_than_cut_to_pieces() {
    let line: Vec<(f64, f64)> = vec![(0.0, 0.0), (1_000_000.0, 0.0)];
    let pieces = dashed(&line, &[], &[1.0, 1.0], 0.0);
    assert_eq!(pieces.len(), 1, "one solid piece, not half a million");
    assert_eq!(pieces[0].0, line);
}

/// A tapered outline is full width in the middle and narrows to a point
/// at each end, a point and not nothing. Dashed, each dash keeps the
/// width the outline has where it falls.
#[test]
fn a_tapered_stroke_narrows_to_a_point_and_its_dashes_follow() {
    let end = |x: f64| Vertex {
        pos: (x, 20.5),
        tan_in: (0.0, 0.0),
        tan_out: (0.0, 0.0),
    };
    let mut it = item(BezierPath {
        vertices: vec![end(10.0), end(90.0)],
        closed: false,
    });
    it.fill = None;
    it.stroke = Some(colour::still(LinearColour([0.0, 1.0, 0.0, 1.0])));
    it.stroke_width = Property::fixed(12.0);
    it.taper = Some(Taper {
        start: Property::fixed(100.0),
        end: Property::fixed(100.0),
        start_length: Property::fixed(50.0),
        end_length: Property::fixed(50.0),
        ease: Property::zero(),
        scope: 0,
    });
    let wide = |rgba: &[u8], x: u32| (0..40).filter(|&y| alpha_at(rgba, 100, x, y) > 128).count();
    let rgba = rasterise_contents(
        &entries(vec![it.clone()]),
        100,
        40,
        0.0,
        0.0,
        100.0,
        40.0,
        0.0,
    );
    assert!(wide(&rgba, 50) >= 11, "full width in the middle");
    assert!(
        wide(&rgba, 20) < wide(&rgba, 35) && wide(&rgba, 35) < wide(&rgba, 50),
        "narrower towards the start"
    );
    assert!(alpha_at(&rgba, 100, 11, 20) > 0, "a point at the very end");
    assert_eq!(wide(&rgba, 5), 0, "and nothing past it");

    it.dashes = vec![Property::fixed(10.0), Property::fixed(10.0)];
    let rgba = rasterise_contents(&entries(vec![it]), 100, 40, 0.0, 0.0, 100.0, 40.0, 0.0);
    assert_eq!(wide(&rgba, 25), 0, "a gap");
    assert!(
        wide(&rgba, 15) > 0 && wide(&rgba, 15) < wide(&rgba, 45),
        "a dash near the end is thinner than one near the middle"
    );
}

/// A linear ramp runs from the fill at one point to the gradient colour at
/// the other, and everything between is a mix of the two.
#[test]
fn a_linear_gradient_ramps_across_the_fill() {
    let mut it = item(square(0.0, 0.0, 20.0));
    it.fill = Some(colour::still(LinearColour([1.0, 0.0, 0.0, 1.0])));
    it.gradient = 1;
    it.gradient_colour = Some(LinearColour([0.0, 0.0, 1.0, 1.0]));
    it.gradient_start_x = Property::fixed(0.0);
    it.gradient_end_x = Property::fixed(20.0);
    let rgba = rasterise_contents(&entries(vec![it]), 20, 20, 0.0, 0.0, 20.0, 20.0, 0.0);

    let left = rgb_at(&rgba, 20, 0, 10);
    let right = rgb_at(&rgba, 20, 19, 10);
    let middle = rgb_at(&rgba, 20, 10, 10);
    assert!(
        left[0] > 200 && left[2] < 60,
        "the fill at the start: {left:?}"
    );
    assert!(
        right[2] > 200 && right[0] < 60,
        "the gradient colour at the end: {right:?}"
    );
    assert!(
        middle[0] > 40 && middle[2] > 40,
        "and a mix of the two between: {middle:?}"
    );
    // The ramp runs *across* the shape, not down it.
    assert_eq!(
        rgb_at(&rgba, 20, 10, 2),
        middle,
        "no ramp along the other axis"
    );
}

/// Radial measures how far *out* you are, the end point sitting on the
/// outer edge, so the middle is the fill and every direction ramps away.
#[test]
fn a_radial_gradient_ramps_out_from_its_start() {
    let mut it = item(square(0.0, 0.0, 20.0));
    it.fill = Some(colour::still(LinearColour([1.0, 0.0, 0.0, 1.0])));
    it.gradient = 2;
    it.gradient_colour = Some(LinearColour([0.0, 0.0, 1.0, 1.0]));
    it.gradient_start_x = Property::fixed(10.0);
    it.gradient_start_y = Property::fixed(10.0);
    it.gradient_end_x = Property::fixed(20.0);
    it.gradient_end_y = Property::fixed(10.0);
    let rgba = rasterise_contents(&entries(vec![it]), 20, 20, 0.0, 0.0, 20.0, 20.0, 0.0);

    assert!(rgb_at(&rgba, 20, 10, 10)[0] > 200, "the fill in the middle");
    // Every direction ramps the same way, which is what makes it radial.
    let out = [(0u32, 10u32), (19, 10), (10, 0), (10, 19)];
    for (x, y) in out {
        assert!(
            rgb_at(&rgba, 20, x, y)[2] > 150,
            "the far colour at the edge ({x}, {y})"
        );
    }
}

/// An end faded to nothing is clear there, and the colour stays the same
/// on the way.
#[test]
fn a_gradient_end_fades_to_clear() {
    let mut it = item(square(0.0, 0.0, 20.0));
    it.fill = Some(colour::still(LinearColour([1.0, 0.0, 0.0, 1.0])));
    it.gradient = 1;
    it.gradient_colour = Some(LinearColour([1.0, 0.0, 0.0, 1.0]));
    it.gradient_start_x = Property::fixed(0.0);
    it.gradient_end_x = Property::fixed(20.0);
    it.gradient_end_opacity = Property::fixed(0.0);
    let rgba = rasterise_contents(
        &entries(vec![it.clone()]),
        20,
        20,
        0.0,
        0.0,
        20.0,
        20.0,
        0.0,
    );

    assert!(alpha_at(&rgba, 20, 0, 10) > 240, "solid at the start");
    assert!(alpha_at(&rgba, 20, 19, 10) < 15, "clear at the end");
    let middle = alpha_at(&rgba, 20, 10, 10);
    assert!((100..160).contains(&middle), "half way between: {middle}");
    assert!(rgb_at(&rgba, 20, 10, 10)[0] > 200, "and still the colour");

    // The other end fades the same way, so the ramp can run clear to solid.
    it.gradient_end_opacity = Property::fixed(100.0);
    it.gradient_start_opacity = Property::fixed(0.0);
    let rgba = rasterise_contents(&entries(vec![it]), 20, 20, 0.0, 0.0, 20.0, 20.0, 0.0);
    assert!(alpha_at(&rgba, 20, 0, 10) < 15, "clear at the start");
    assert!(alpha_at(&rgba, 20, 19, 10) > 240, "solid at the end");
}

/// A stop between the ends is the colour at its own place, and the ramp
/// runs to it and away from it.
#[test]
fn a_gradient_passes_through_the_stops_between_its_ends() {
    let mut it = item(square(0.0, 0.0, 20.0));
    it.fill = Some(colour::still(LinearColour([1.0, 0.0, 0.0, 1.0])));
    it.gradient = 1;
    it.gradient_colour = Some(LinearColour([0.0, 0.0, 1.0, 1.0]));
    it.gradient_start_x = Property::fixed(0.0);
    it.gradient_end_x = Property::fixed(20.0);
    it.gradient_stops = vec![GradientStop {
        position: Property::fixed(50.0),
        colour: LinearColour([0.0, 1.0, 0.0, 1.0]),
        opacity: Property::fixed(0.0),
    }];
    let rgba = rasterise_contents(
        &entries(vec![it.clone()]),
        20,
        20,
        0.0,
        0.0,
        20.0,
        20.0,
        0.0,
    );
    assert!(rgb_at(&rgba, 20, 0, 10)[0] > 200, "the fill at the start");
    assert!(
        rgb_at(&rgba, 20, 19, 10)[2] > 200,
        "the far colour at the end"
    );
    assert!(alpha_at(&rgba, 20, 10, 10) < 40, "and clear at the stop");

    // Keyed solid, the stop is its own colour where it sits by then.
    let key = |secs: i64, value: f64| crate::anim::Keyframe {
        time: crate::time::Rational::new(secs, 1).expect("a whole second"),
        value,
        interp_in: crate::anim::SideInterp::Linear,
        interp_out: crate::anim::SideInterp::Linear,
    };
    it.gradient_stops[0].opacity.animation =
        crate::anim::Animation::Keyframed(vec![key(0, 0.0), key(1, 100.0)]);
    let rgba = rasterise_contents(&entries(vec![it]), 20, 20, 0.0, 0.0, 20.0, 20.0, 1.0);
    let middle = rgb_at(&rgba, 20, 10, 10);
    assert!(
        middle[1] > 200 && middle[0] < 120 && middle[2] < 120,
        "green in the middle: {middle:?}"
    );
}

/// The outline pushed out of the path: art where the path is not, filled
/// as one piece.
#[test]
fn an_offset_path_grows_the_shape_and_a_negative_one_shrinks_it() {
    let grown = rasterise_contents(
        &with(item(square(5.0, 5.0, 10.0)), vec![offset(3.0)]),
        20,
        20,
        0.0,
        0.0,
        20.0,
        20.0,
        0.0,
    );
    assert_eq!(
        alpha_at(&grown, 20, 3, 10),
        255,
        "outside the path, inside the offset"
    );
    assert_eq!(alpha_at(&grown, 20, 0, 10), 0, "and not beyond it");

    let shrunk = rasterise_contents(
        &with(item(square(5.0, 5.0, 10.0)), vec![offset(-3.0)]),
        20,
        20,
        0.0,
        0.0,
        20.0,
        20.0,
        0.0,
    );
    assert_eq!(
        alpha_at(&shrunk, 20, 6, 10),
        0,
        "inside the path, outside the offset"
    );
    assert_eq!(
        alpha_at(&shrunk, 20, 10, 10),
        255,
        "and the middle is still there"
    );
}

/// A repeated square is drawn where the step puts it, and nowhere else.
#[test]
fn a_repeater_draws_a_copy_at_every_step() {
    let contents = with(
        item(square(0.0, 0.0, 6.0)),
        vec![ModifierKind::Repeat(repeater(3.0, 10.0))],
    );
    // The box has grown to hold all three, so it is asked for explicitly.
    let rgba = rasterise_contents(&contents, 40, 10, 0.0, 0.0, 40.0, 10.0, 0.0);
    for x in [3, 13, 23] {
        assert_eq!(alpha_at(&rgba, 40, x, 3), 255, "a copy at {x}");
    }
    for x in [8, 18, 33] {
        assert_eq!(alpha_at(&rgba, 40, x, 3), 0, "the gap at {x}");
    }
}

/// A count nobody could draw is held at the ceiling rather than refused,
/// and a fractional one is a count of things, so it rounds.
#[test]
fn the_copy_count_is_held_at_the_ceiling_and_never_fractional() {
    let copies = |n: f64| {
        let ShapeEntry::Modifier(m) = modifier(ModifierKind::Repeat(repeater(n, 1.0))) else {
            unreachable!()
        };
        modifier::copies_at(&[&m], 0.0).len()
    };
    assert_eq!(copies(1e9), modifier::MAX_COPIES as usize);
    assert_eq!(copies(-4.0), 1, "never fewer than the original");
    assert_eq!(copies(2.6), 3);
}

/// A modifier acts on the art above it, two of one kind stack, a group keeps
/// its modifiers to itself, and an older file's fields lift into just that.
#[test]
fn modifiers_act_on_the_art_above_them_and_stack() {
    let drawn = |contents: &[ShapeEntry]| {
        let rgba = rasterise_contents(contents, 40, 40, 0.0, 0.0, 40.0, 40.0, 0.0);
        (0..40u32)
            .flat_map(|y| (0..40u32).map(move |x| (x, y)))
            .filter(|&(x, y)| alpha_at(&rgba, 40, x, y) > 0)
            .count()
    };
    let (a, b) = (item(square(0.0, 0.0, 4.0)), item(square(0.0, 10.0, 4.0)));
    let step = || modifier(ModifierKind::Repeat(repeater(2.0, 10.0)));

    // Below both items, the repeater draws both twice.
    let both = vec![
        ShapeEntry::Item(a.clone()),
        ShapeEntry::Item(b.clone()),
        step(),
    ];
    assert_eq!(drawn(&both), 4 * 16);

    // Between them, it reaches only the one above it.
    let one = vec![
        ShapeEntry::Item(a.clone()),
        step(),
        ShapeEntry::Item(b.clone()),
    ];
    assert_eq!(drawn(&one), 3 * 16);

    // Stacked, the second repeats the copies: a grid.
    let grid = vec![
        ShapeEntry::Item(a.clone()),
        step(),
        modifier(ModifierKind::Repeat(repeater(3.0, 0.0))),
    ];
    let art = modifier::lowered(&grid);
    assert_eq!(modifier::copies_at(&art[0].mods, 0.0).len(), 6);

    // In a group, it reaches nothing outside the group.
    let grouped = vec![
        ShapeEntry::Item(a.clone()),
        ShapeEntry::Group(modifier::ShapeGroup {
            id: Uuid::now_v7(),
            name: "Group".into(),
            entries: vec![ShapeEntry::Item(b.clone()), step()],
            extra: serde_json::Map::new(),
        }),
    ];
    assert_eq!(drawn(&grouped), 3 * 16);

    // An older file's fields become that very group, and a first item's
    // fields stay flat, where there is nothing above them to reach.
    let mut old_a = serde_json::to_value(&a).unwrap();
    old_a["repeat_copies"] = serde_json::json!(2.0);
    old_a["repeat_position_x"] = serde_json::json!(10.0);
    let mut old_b = serde_json::to_value(&b).unwrap();
    old_b["repeat_copies"] = serde_json::json!(2.0);
    old_b["repeat_position_x"] = serde_json::json!(10.0);
    let lifted: Vec<ShapeEntry> =
        modifier::read_entries(serde_json::json!([old_a, old_b])).unwrap();
    assert!(matches!(
        lifted.as_slice(),
        [ShapeEntry::Item(_), ShapeEntry::Modifier(_), ShapeEntry::Group(g)] if g.entries.len() == 2
    ));
    assert_eq!(drawn(&lifted), 4 * 16);
    // Lifted twice, the same ids both times, and the new layout written out
    // carries no old field.
    let again = modifier::read_entries(serde_json::to_value(&lifted).unwrap()).unwrap();
    assert_eq!(lifted, again);
    let written = serde_json::to_string(&lifted).unwrap();
    assert!(!written.contains("repeat_copies"), "{written}");
    assert!(written.contains("\"modifier\":\"repeat\""), "{written}");
}

/// Two overlapping squares, A at 0..20 and B at 10..30, combined by `kind`
/// and drawn into their shared 30×30 box. The three probes are the corner
/// only A covers, the middle both cover, and the corner only B covers.
fn combined(kind: u32) -> (u8, u8, u8) {
    let mut b = item(square(10.0, 10.0, 20.0));
    b.combine = kind;
    b.fill = Some(colour::still(LinearColour([0.0, 1.0, 0.0, 1.0])));
    let contents = entries(vec![item(square(0.0, 0.0, 20.0)), b]);
    let rgba = rasterise_contents(&contents, 30, 30, 0.0, 0.0, 30.0, 30.0, 0.0);
    (
        alpha_at(&rgba, 30, 5, 5),
        alpha_at(&rgba, 30, 15, 15),
        alpha_at(&rgba, 30, 25, 25),
    )
}

#[test]
fn an_intersect_keeps_only_what_both_paths_covered() {
    assert_eq!(combined(3), (0, 255, 0));
}

#[test]
fn an_exclude_keeps_what_exactly_one_path_covered() {
    assert_eq!(combined(4), (255, 0, 255));
}

#[test]
fn a_subtracted_hole_is_a_hole_and_the_ring_round_it_is_filled() {
    // The case a single contour cannot express: one square with another
    // taken out of its middle is two rings, and the fill has to count the
    // crossings across both at once.
    let mut hole = item(square(10.0, 10.0, 10.0));
    hole.combine = 2;
    let contents = entries(vec![item(square(0.0, 0.0, 30.0)), hole]);
    let rgba = rasterise_contents(&contents, 30, 30, 0.0, 0.0, 30.0, 30.0, 0.0);
    assert_eq!(alpha_at(&rgba, 30, 15, 15), 0, "the middle is empty");
    for (x, y) in [(5, 5), (25, 25), (5, 25), (25, 5)] {
        assert_eq!(alpha_at(&rgba, 30, x, y), 255, "the ring at {x},{y}");
    }
}

#[test]
fn a_path_that_crosses_itself_stays_filled_where_it_crosses() {
    // A star drawn in one stroke goes round its middle twice. Even-odd
    // left that middle hollow, After Effects fills it.
    let vertices = (0..5)
        .map(|i| {
            let a = (f64::from(i) * 144.0 - 90.0).to_radians();
            Vertex {
                pos: (15.0 + 14.0 * a.cos(), 15.0 + 14.0 * a.sin()),
                tan_in: (0.0, 0.0),
                tan_out: (0.0, 0.0),
            }
        })
        .collect();
    let star = BezierPath {
        vertices,
        closed: true,
    };
    let rgba = rasterise_contents(
        &entries(vec![item(star)]),
        30,
        30,
        0.0,
        0.0,
        30.0,
        30.0,
        0.0,
    );
    assert_eq!(alpha_at(&rgba, 30, 15, 15), 255, "the middle is filled");
}

#[test]
fn a_run_folds_left_to_right_so_three_items_read_in_order() {
    // (A ∪ B) − C: the union covers 0..30, and C takes 10..20 back out.
    let mut b = item(square(15.0, 0.0, 15.0));
    b.combine = 1;
    let mut c = item(square(10.0, 0.0, 10.0));
    c.combine = 2;
    let contents = entries(vec![item(square(0.0, 0.0, 15.0)), b, c]);
    let rgba = rasterise_contents(&contents, 30, 10, 0.0, 0.0, 30.0, 10.0, 0.0);
    assert_eq!(alpha_at(&rgba, 30, 5, 5), 255, "A survives");
    assert_eq!(alpha_at(&rgba, 30, 15, 5), 0, "C cut the middle out");
    assert_eq!(alpha_at(&rgba, 30, 25, 5), 255, "B survives");
}

#[test]
fn combining_a_shape_is_deterministic() {
    let mut b = item(square(3.0, 3.0, 9.0));
    b.combine = 2;
    let contents = entries(vec![item(square(0.0, 0.0, 10.0)), b]);
    let once = rasterise_contents(&contents, 16, 16, 0.0, 0.0, 16.0, 16.0, 0.0);
    let twice = rasterise_contents(&contents, 16, 16, 0.0, 0.0, 16.0, 16.0, 0.0);
    assert_eq!(once, twice);
}

fn shape_key(secs: i64, path: &BezierPath) -> crate::mask::PathKeyframe {
    crate::mask::PathKeyframe {
        time: crate::time::Rational::new(secs, 1).expect("a whole second"),
        path: path.clone(),
        interp_in: crate::anim::SideInterp::Linear,
        interp_out: crate::anim::SideInterp::Linear,
    }
}

#[test]
fn the_morph_is_what_the_picture_and_the_box_both_read() {
    let (from, to) = (square(0.0, 0.0, 10.0), square(20.0, 0.0, 10.0));
    let mut it = item(from.clone());
    it.path_keys = vec![shape_key(0, &from), shape_key(2, &to)];

    // The box follows the shape being drawn, not the still path.
    assert_eq!(it.bounds(&[], 0.0), Some((0.0, 0.0, 10.0, 10.0)));
    assert_eq!(it.bounds(&[], 2.0), Some((20.0, 0.0, 30.0, 10.0)));
    assert_eq!(it.bounds(&[], 1.0), Some((10.0, 0.0, 20.0, 10.0)));

    // And so does the ink: at the far key the art is at the far end of the
    // box, with nothing left where it started.
    let contents = entries(vec![it]);
    let rgba = rasterise_contents(&contents, 30, 10, 0.0, 0.0, 30.0, 10.0, 2.0);
    assert_eq!(alpha_at(&rgba, 30, 25, 5), 255, "drawn where the key says");
    assert_eq!(alpha_at(&rgba, 30, 5, 5), 0, "and nowhere else");
}

#[test]
fn an_unkeyed_shape_is_absent_from_the_file() {
    let json = serde_json::to_string(&item(square(0.0, 0.0, 4.0))).unwrap();
    assert!(!json.contains("path_keys"), "{json}");
    assert!(!json.contains("gradient"), "{json}");

    let mut keyed = item(square(0.0, 0.0, 4.0));
    keyed.path_keys = vec![shape_key(1, &square(2.0, 2.0, 4.0))];
    // A faded gradient with a stop between its ends comes back as it went.
    keyed.gradient = 1;
    keyed.gradient_end_opacity = Property::fixed(0.0);
    keyed.gradient_stops = vec![GradientStop {
        position: Property::fixed(40.0),
        colour: LinearColour([0.0, 1.0, 0.0, 1.0]),
        opacity: Property::fixed(50.0),
    }];
    let json = serde_json::to_string(&keyed).unwrap();
    assert!(json.contains("path_keys"), "{json}");
    let back: ShapeItem = serde_json::from_str(&json).unwrap();
    assert_eq!(back.path_keys.len(), 1);
    assert_eq!(back, keyed);
}

/// A parametric rectangle draws the rectangle its numbers make, and a keyed
/// width moves it, frame by frame.
#[test]
fn a_parametric_rectangle_draws_its_numbers_and_a_keyed_width_moves_it() {
    use crate::anim::{Animation, Keyframe, SideInterp};
    use crate::time::Rational;
    let key = |secs: i64, value: f64| Keyframe {
        time: Rational::new(secs, 1).unwrap(),
        value,
        interp_in: SideInterp::Linear,
        interp_out: SideInterp::Linear,
    };
    let mut made = item(square(0.0, 0.0, 1.0));
    made.primitive = Some(primitive::ShapePrimitive {
        kind: 0,
        position_x: Property::fixed(50.0),
        position_y: Property::fixed(50.0),
        width: Property {
            animation: Animation::Keyframed(vec![key(0, 40.0), key(1, 80.0)]),
            extra: serde_json::Map::new(),
        },
        height: Property::fixed(40.0),
        roundness: Property::zero(),
        points: Property::fixed(5.0),
        rotation: Property::zero(),
        outer_radius: Property::zero(),
        inner_radius: Property::zero(),
        outer_roundness: Property::zero(),
        inner_roundness: Property::zero(),
        extra: serde_json::Map::new(),
    });
    let contents = entries(vec![made]);
    assert!(modifier::items(&contents)[0].path_is_animated());
    assert_eq!(
        contents_bounds(&contents, 0.0),
        Some((30.0, 30.0, 70.0, 70.0))
    );
    assert_eq!(
        contents_bounds(&contents, 1.0),
        Some((10.0, 30.0, 90.0, 70.0))
    );
    let drawn = |c: &[ShapeEntry]| rasterise_contents(c, 100, 100, 0.0, 0.0, 100.0, 100.0, 0.0);
    assert_eq!(
        drawn(&contents),
        drawn(&entries(vec![item(
            Mask::rectangle(30.0, 30.0, 40.0, 40.0).path
        )]))
    );
}

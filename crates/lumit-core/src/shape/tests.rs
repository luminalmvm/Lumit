use super::geometry::dist;
use super::raster::{dashed, Pressed, TAPER_TIP};
use super::*;
use crate::mask::{Mask, Vertex};

fn square(x: f64, y: f64, side: f64) -> BezierPath {
    Mask::rectangle(x, y, side, side).path
}

fn item(path: BezierPath) -> ShapeItem {
    ShapeItem::filled("Rectangle", path, LinearColour([1.0, 0.0, 0.0, 1.0]))
}

fn alpha_at(rgba: &[u8], w: u32, x: u32, y: u32) -> u8 {
    rgba[((y * w + x) as usize) * 4 + 3]
}

fn rgb_at(rgba: &[u8], w: u32, x: u32, y: u32) -> [u8; 3] {
    let i = ((y * w + x) as usize) * 4;
    [rgba[i], rgba[i + 1], rgba[i + 2]]
}

#[test]
fn a_layers_size_is_the_box_its_art_fills() {
    let contents = vec![item(square(10.0, 20.0, 30.0))];
    assert_eq!(
        contents_bounds(&contents, 0.0),
        Some((10.0, 20.0, 40.0, 50.0))
    );

    // Two items: the box holds both.
    let contents = vec![item(square(10.0, 20.0, 30.0)), item(square(-5.0, 0.0, 8.0))];
    assert_eq!(
        contents_bounds(&contents, 0.0),
        Some((-5.0, 0.0, 40.0, 50.0))
    );

    assert!(contents_bounds(&[], 0.0).is_none());
}

#[test]
fn a_filled_shape_draws_its_colour_inside_and_nothing_outside() {
    let contents = vec![item(square(0.0, 0.0, 20.0))];
    // Rasterised into its own bounding box at 1:1.
    let rgba = rasterise_contents(&contents, 20, 20, 0.0, 0.0, 20.0, 20.0, 0.0);
    assert_eq!(alpha_at(&rgba, 20, 10, 10), 255, "inside the square");
    assert_eq!(rgb_at(&rgba, 20, 10, 10), [255, 0, 0]);

    // A smaller square in a bigger box leaves the rest transparent.
    let contents = vec![item(square(5.0, 5.0, 5.0))];
    let rgba = rasterise_contents(&contents, 20, 20, 0.0, 0.0, 20.0, 20.0, 0.0);
    assert_eq!(alpha_at(&rgba, 20, 7, 7), 255);
    assert_eq!(alpha_at(&rgba, 20, 18, 18), 0, "outside the art");
}

#[test]
fn a_fill_and_a_stroke_are_both_drawn_and_the_stroke_is_on_top() {
    let mut it = item(square(4.0, 4.0, 12.0));
    it.stroke = Some(colour::still(LinearColour([0.0, 1.0, 0.0, 1.0])));
    it.stroke_width = Property::fixed(3.0);
    let rgba = rasterise_contents(&[it], 20, 20, 0.0, 0.0, 20.0, 20.0, 0.0);

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
    let rgba = rasterise_contents(&[under, over], 20, 20, 0.0, 0.0, 20.0, 20.0, 0.0);
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
    assert!(empty.bounds(0.0).is_none());
    assert!(rasterise_contents(&[empty], 4, 4, 0.0, 0.0, 4.0, 4.0, 0.0)
        .iter()
        .all(|&b| b == 0));
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
    let mut it = item(square(0.0, 0.0, 20.0));
    it.trim_end = Property::fixed(50.0);
    let half = it.trimmed_at(0.0).expect("a trimmed piece");
    let whole = length(&flatten_path(&it.path));
    assert!(
        (length(&half) - whole / 2.0).abs() < 1e-6,
        "half the perimeter, got {} of {whole}",
        length(&half)
    );
    // It starts where the path starts.
    assert_eq!(half.first().copied(), Some(it.path.vertices[0].pos));
}

#[test]
fn a_keyed_trim_is_read_on_the_layers_clock() {
    let mut it = item(square(0.0, 0.0, 20.0));
    let key = |secs: i64, value: f64| crate::anim::Keyframe {
        time: crate::time::Rational::new(secs, 1).expect("a whole second"),
        value,
        interp_in: crate::anim::SideInterp::Linear,
        interp_out: crate::anim::SideInterp::Linear,
    };
    it.trim_end.animation = crate::anim::Animation::Keyframed(vec![key(0, 0.0), key(1, 100.0)]);
    let at = |t: f64| {
        ink(&rasterise_contents(
            &[it.clone()],
            20,
            20,
            0.0,
            0.0,
            20.0,
            20.0,
            t,
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
    let at = |t: f64| {
        rgb_at(
            &rasterise_contents(
                std::slice::from_ref(&keyed),
                20,
                20,
                0.0,
                0.0,
                20.0,
                20.0,
                t,
            ),
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
    let rgba = rasterise_contents(&[it.clone()], 100, 40, 0.0, 0.0, 100.0, 40.0, 0.0);
    assert!(wide(&rgba, 50) >= 11, "full width in the middle");
    assert!(
        wide(&rgba, 20) < wide(&rgba, 35) && wide(&rgba, 35) < wide(&rgba, 50),
        "narrower towards the start"
    );
    assert!(alpha_at(&rgba, 100, 11, 20) > 0, "a point at the very end");
    assert_eq!(wide(&rgba, 5), 0, "and nothing past it");

    it.dashes = vec![Property::fixed(10.0), Property::fixed(10.0)];
    let rgba = rasterise_contents(&[it], 100, 40, 0.0, 0.0, 100.0, 40.0, 0.0);
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
    let rgba = rasterise_contents(&[it], 20, 20, 0.0, 0.0, 20.0, 20.0, 0.0);

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
/// outer edge — so the middle is the fill and every direction ramps away.
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
    let rgba = rasterise_contents(&[it], 20, 20, 0.0, 0.0, 20.0, 20.0, 0.0);

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
    let rgba = rasterise_contents(&[it.clone()], 20, 20, 0.0, 0.0, 20.0, 20.0, 0.0);

    assert!(alpha_at(&rgba, 20, 0, 10) > 240, "solid at the start");
    assert!(alpha_at(&rgba, 20, 19, 10) < 15, "clear at the end");
    let middle = alpha_at(&rgba, 20, 10, 10);
    assert!((100..160).contains(&middle), "half way between: {middle}");
    assert!(rgb_at(&rgba, 20, 10, 10)[0] > 200, "and still the colour");

    // The other end fades the same way, so the ramp can run clear to solid.
    it.gradient_end_opacity = Property::fixed(100.0);
    it.gradient_start_opacity = Property::fixed(0.0);
    let rgba = rasterise_contents(&[it], 20, 20, 0.0, 0.0, 20.0, 20.0, 0.0);
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
    let rgba = rasterise_contents(&[it.clone()], 20, 20, 0.0, 0.0, 20.0, 20.0, 0.0);
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
    let rgba = rasterise_contents(&[it], 20, 20, 0.0, 0.0, 20.0, 20.0, 1.0);
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
    let mut it = item(square(5.0, 5.0, 10.0));
    it.offset_amount = Property::fixed(3.0);
    let grown = rasterise_contents(&[it.clone()], 20, 20, 0.0, 0.0, 20.0, 20.0, 0.0);
    assert_eq!(
        alpha_at(&grown, 20, 3, 10),
        255,
        "outside the path, inside the offset"
    );
    assert_eq!(alpha_at(&grown, 20, 0, 10), 0, "and not beyond it");

    it.offset_amount = Property::fixed(-3.0);
    let shrunk = rasterise_contents(&[it], 20, 20, 0.0, 0.0, 20.0, 20.0, 0.0);
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
    let mut it = item(square(0.0, 0.0, 6.0));
    it.repeat_copies = Property::fixed(3.0);
    it.repeat_position_x = Property::fixed(10.0);
    let contents = vec![it];
    // The box has grown to hold all three, so it is asked for explicitly.
    let rgba = rasterise_contents(&contents, 40, 10, 0.0, 0.0, 40.0, 10.0, 0.0);
    for x in [3, 13, 23] {
        assert_eq!(alpha_at(&rgba, 40, x, 3), 255, "a copy at {x}");
    }
    for x in [8, 18, 33] {
        assert_eq!(alpha_at(&rgba, 40, x, 3), 0, "the gap at {x}");
    }
}

/// A count nobody could draw is held at the ceiling rather than refused —
/// and a fractional one is a count of things, so it rounds.
#[test]
fn the_copy_count_is_held_at_the_ceiling_and_never_fractional() {
    let mut it = item(square(0.0, 0.0, 2.0));
    it.repeat_position_x = Property::fixed(1.0);
    it.repeat_copies = Property::fixed(1e9);
    assert_eq!(it.copies_at(0.0).len(), MAX_COPIES as usize);
    it.repeat_copies = Property::fixed(-4.0);
    assert_eq!(it.copies_at(0.0).len(), 1, "never fewer than the original");
    it.repeat_copies = Property::fixed(2.6);
    assert_eq!(it.copies_at(0.0).len(), 3);
}

/// Two overlapping squares — A at 0..20, B at 10..30 — combined by `kind`
/// and drawn into their shared 30×30 box. The three probes are the corner
/// only A covers, the middle both cover, and the corner only B covers.
fn combined(kind: u32) -> (u8, u8, u8) {
    let mut b = item(square(10.0, 10.0, 20.0));
    b.combine = kind;
    b.fill = Some(colour::still(LinearColour([0.0, 1.0, 0.0, 1.0])));
    let contents = vec![item(square(0.0, 0.0, 20.0)), b];
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
    let contents = vec![item(square(0.0, 0.0, 30.0)), hole];
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
    let rgba = rasterise_contents(&[item(star)], 30, 30, 0.0, 0.0, 30.0, 30.0, 0.0);
    assert_eq!(alpha_at(&rgba, 30, 15, 15), 255, "the middle is filled");
}

#[test]
fn a_run_folds_left_to_right_so_three_items_read_in_order() {
    // (A ∪ B) − C: the union covers 0..30, and C takes 10..20 back out.
    let mut b = item(square(15.0, 0.0, 15.0));
    b.combine = 1;
    let mut c = item(square(10.0, 0.0, 10.0));
    c.combine = 2;
    let contents = vec![item(square(0.0, 0.0, 15.0)), b, c];
    let rgba = rasterise_contents(&contents, 30, 10, 0.0, 0.0, 30.0, 10.0, 0.0);
    assert_eq!(alpha_at(&rgba, 30, 5, 5), 255, "A survives");
    assert_eq!(alpha_at(&rgba, 30, 15, 5), 0, "C cut the middle out");
    assert_eq!(alpha_at(&rgba, 30, 25, 5), 255, "B survives");
}

#[test]
fn combining_a_shape_is_deterministic() {
    let mut b = item(square(3.0, 3.0, 9.0));
    b.combine = 2;
    let contents = vec![item(square(0.0, 0.0, 10.0)), b];
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
    assert_eq!(it.bounds(0.0), Some((0.0, 0.0, 10.0, 10.0)));
    assert_eq!(it.bounds(2.0), Some((20.0, 0.0, 30.0, 10.0)));
    assert_eq!(it.bounds(1.0), Some((10.0, 0.0, 20.0, 10.0)));

    // And so does the ink: at the far key the art is at the far end of the
    // box, with nothing left where it started.
    let contents = vec![it];
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

fn circle() -> BezierPath {
    Mask::ellipse(50.0, 50.0, 40.0, 40.0).path
}

fn twisted(angle: f64, axis: f64, centre: (f64, f64)) -> ShapeItem {
    let mut it = item(circle());
    it.twist = Some(Twist {
        angle: Property::fixed(angle),
        rotation: Property::zero(),
        axis: Property::fixed(axis),
        centre_x: Property::fixed(centre.0),
        centre_y: Property::fixed(centre.1),
    });
    it
}

fn drawn(it: &ShapeItem) -> Vec<u8> {
    rasterise_contents(
        std::slice::from_ref(it),
        100,
        100,
        0.0,
        0.0,
        100.0,
        100.0,
        0.0,
    )
}

/// The farthest and nearest any point is from the circle's centre.
fn radii(points: &[(f64, f64)]) -> (f64, f64) {
    points
        .iter()
        .map(|&p| dist(p, (50.0, 50.0)))
        .fold((f64::MAX, 0.0f64), |(lo, hi), r| (lo.min(r), hi.max(r)))
}

#[test]
fn a_twist_of_0_does_nothing_and_a_half_turn_makes_a_circle_a_figure_eight() {
    let same = |x: &[(f64, f64)], y: &[(f64, f64)]| {
        assert_eq!(x.len(), y.len());
        for (a, b) in x.iter().zip(y) {
            assert!(dist(*a, *b) < 1e-9, "{a:?} against {b:?}");
        }
    };
    let turned = |angle: f64, rotation: f64| {
        let mut it = twisted(angle, 30.0, (20.0, 70.0));
        if let Some(w) = &mut it.twist {
            w.rotation = Property::fixed(rotation);
        }
        it.trimmed_at(0.0).unwrap()
    };
    // At 0 and 0 every point is where it was, wherever the axis is.
    same(&flatten_fine(&circle()), &turned(0.0, 0.0));
    // A full turn of the rotation comes back to where it started.
    same(&turned(180.0, 0.0), &turned(180.0, 360.0));

    // At 180 the middle is pinched to nothing, with a lobe either side.
    let eight = drawn(&twisted(180.0, 0.0, (50.0, 50.0)));
    assert_eq!(alpha_at(&eight, 100, 50, 25), 0, "the eight crosses here");
    assert_eq!(alpha_at(&eight, 100, 30, 50), 255, "a lobe either side");
    assert_eq!(alpha_at(&eight, 100, 70, 50), 255);

    // An upright axis pinches across the other way.
    let upright = drawn(&twisted(180.0, 90.0, (50.0, 50.0)));
    assert_eq!(alpha_at(&upright, 100, 25, 50), 0);
    assert_eq!(alpha_at(&upright, 100, 50, 30), 255);
}

fn deformed(mode: u32, amount: f64, seed: f64, evolution: f64, detail: f64) -> ShapeItem {
    let mut it = item(circle());
    it.deform = Some(Deform {
        mode,
        amount: Property::fixed(amount),
        scale: Property::fixed(10.0),
        evolution: Property::fixed(evolution),
        seed: Property::fixed(seed),
        detail: Property::fixed(detail),
    });
    it
}

/// How many times the distance from the centre turns round along `points`.
fn bends(points: &[(f64, f64)]) -> usize {
    let r: Vec<f64> = points.iter().map(|&p| dist(p, (50.0, 50.0))).collect();
    r.windows(3)
        .filter(|w| (w[1] - w[0]) * (w[2] - w[1]) < 0.0)
        .count()
}

#[test]
fn a_deform_moves_the_outline_no_further_than_its_amount_and_repeats_itself() {
    let plain = drawn(&item(circle()));
    let none = deformed(0, 0.0, 1.0, 0.0, 4.0);
    assert!(none.trimmed_at(0.0).is_none(), "nothing to bend");
    assert_eq!(drawn(&none), plain);

    // Displace pushes along the normal by up to the amount in pixels.
    let pushed = deformed(0, 8.0, 1.0, 0.0, 4.0).trimmed_at(0.0).unwrap();
    let (lo, hi) = radii(&pushed);
    assert!(lo < 39.0 && hi > 41.0, "moved both ways: {lo}..{hi}");
    assert!(lo > 31.9 && hi < 48.1, "and within the amount: {lo}..{hi}");

    // Scale moves each point from the middle by up to the amount per cent.
    let scaled = deformed(1, 50.0, 1.0, 0.0, 4.0).trimmed_at(0.0).unwrap();
    let (lo, hi) = radii(&scaled);
    assert!(lo < 39.0 && hi > 41.0, "moved both ways: {lo}..{hi}");
    assert!(lo > 19.9 && hi < 60.1, "and within the amount: {lo}..{hi}");

    // The same seed and evolution are the same outline every time.
    let again = deformed(0, 8.0, 1.0, 0.0, 4.0).trimmed_at(0.0).unwrap();
    assert_eq!(pushed, again);
    assert_ne!(
        pushed,
        deformed(0, 8.0, 2.0, 0.0, 4.0).trimmed_at(0.0).unwrap()
    );
    assert_ne!(
        pushed,
        deformed(0, 8.0, 1.0, 90.0, 4.0).trimmed_at(0.0).unwrap()
    );

    // Little detail bends the outline broadly; more bends it often.
    let broad = deformed(0, 8.0, 1.0, 0.0, 0.1).trimmed_at(0.0).unwrap();
    assert!(
        bends(&broad) * 4 < bends(&pushed),
        "{} against {}",
        bends(&broad),
        bends(&pushed)
    );
}

#[test]
fn a_smooth_rounds_the_path_inside_its_points_and_an_open_one_keeps_its_ends() {
    let mut it = item(square(0.0, 0.0, 40.0));
    it.smooth = Some(Smooth {
        amount: Property::zero(),
    });
    assert!(it.trimmed_at(0.0).is_none(), "nothing to round");

    it.smooth = Some(Smooth {
        amount: Property::fixed(10.0),
    });
    let round = it.trimmed_at(0.0).unwrap();
    assert!(
        round
            .iter()
            .all(|p| (0.0..=40.0).contains(&p.0) && (0.0..=40.0).contains(&p.1)),
        "never past the square"
    );
    let corner = round
        .iter()
        .map(|&p| dist(p, (0.0, 0.0)))
        .fold(f64::MAX, f64::min);
    assert!(corner > 2.0, "the corner is rounded off: {corner}");

    // An open zigzag smooths towards a line and keeps both its ends.
    let zigzag: Vec<(f64, f64)> = (0..=10)
        .map(|i| (i as f64 * 10.0, if i % 2 == 0 { 0.0 } else { 10.0 }))
        .collect();
    let flat = smoothed(&zigzag, false, 15.0);
    assert_eq!(flat.first(), zigzag.first());
    assert!(dist(*flat.last().unwrap(), (100.0, 0.0)) < 1e-9);
    assert!(flat.iter().all(|p| (0.0..=10.0).contains(&p.1)));
    let middle = flat.iter().find(|p| p.0 >= 50.0).unwrap();
    assert!((middle.1 - 5.0).abs() < 2.5, "half flattened: {middle:?}");
    assert_eq!(smoothed(&zigzag, false, 0.0), zigzag);
}

fn taper(start_length: f64, end_length: f64, scope: u32) -> Taper {
    Taper {
        start: Property::fixed(100.0),
        end: Property::fixed(100.0),
        start_length: Property::fixed(start_length),
        end_length: Property::fixed(end_length),
        ease: Property::fixed(50.0),
        scope,
    }
}

#[test]
fn a_taper_never_crosses_its_ends_and_runs_per_path_or_along_the_whole_shape() {
    let line = vec![(0.0, 0.0), (100.0, 0.0)];
    let width = |taper: Taper| taper.applied(vec![line.clone()], 0.0).remove(0).1;

    // Start only: a point at the start, full width at the end.
    let mut start_only = taper(50.0, 50.0, 0);
    start_only.end = Property::zero();
    let w = width(start_only);
    assert!(w[0] <= TAPER_TIP && (w[w.len() - 1] - 1.0).abs() < 1e-9);

    // Two lengths of 80 are 160 per cent of the line, shrunk to half each,
    // so the line still reaches full width in the middle and nowhere else.
    let w = width(taper(80.0, 80.0, 0));
    let widest = w.iter().copied().fold(0.0, f64::max);
    assert!((widest - 1.0).abs() < 1e-6, "full width once: {widest}");
    assert!(w[0] <= TAPER_TIP && w[w.len() - 1] <= TAPER_TIP);
    let peak = w.iter().position(|&x| x == widest).unwrap() as f64 / w.len() as f64;
    assert!((peak - 0.5).abs() < 0.05, "in the middle: {peak}");

    // Two squares joined into one shape: each path tapers on its own, or
    // one taper runs along both.
    let lines = |scope: u32| {
        let mut a = item(square(0.0, 0.0, 20.0));
        a.stroke = Some(colour::still(LinearColour([0.0, 1.0, 0.0, 1.0])));
        a.stroke_width = Property::fixed(4.0);
        a.taper = Some(taper(50.0, 50.0, scope));
        let mut b = item(square(40.0, 0.0, 20.0));
        b.combine = 1;
        let contents = vec![a, b];
        let art = Art::at(&contents, 0..2, 0.0).unwrap();
        art.stroked(&art.placed(&Affine::IDENTITY), 1.0, 0.0)
    };
    let ends = |l: &Pressed| (l.1[0], l.1[l.1.len() - 1]);
    let each = lines(0);
    assert_eq!(each.len(), 2);
    for l in &each {
        let (a, b) = ends(l);
        assert!(a <= TAPER_TIP && b <= TAPER_TIP, "each path to a point");
    }
    let whole = lines(1);
    let (a, b) = ends(&whole[0]);
    assert!(a <= TAPER_TIP && (b - 1.0).abs() < 1e-6, "{a}..{b}");
    let (a, b) = ends(&whole[1]);
    assert!((a - 1.0).abs() < 1e-6 && b <= TAPER_TIP, "{a}..{b}");
}

/// The layer's box has to hold everything a twist or a deform draws.
#[test]
fn the_box_holds_a_twisted_or_deformed_shape() {
    let inside = |it: &ShapeItem| {
        let (x0, y0, x1, y1) = it.bounds(0.0).unwrap();
        let points = it.trimmed_at(0.0).unwrap();
        assert!(
            points
                .iter()
                .all(|p| p.0 >= x0 && p.0 <= x1 && p.1 >= y0 && p.1 <= y1),
            "{:?} outside {:?}",
            points
                .iter()
                .find(|p| p.0 < x0 || p.0 > x1 || p.1 < y0 || p.1 > y1),
            (x0, y0, x1, y1)
        );
        (x0, y0, x1, y1)
    };
    // A square twisted on a slant reaches past its own corners.
    let mut slant = item(square(0.0, 0.0, 20.0));
    slant.twist = twisted(30.0, 45.0, (10.0, 10.0)).twist;
    let (x0, y0, x1, y1) = inside(&slant);
    assert!(x0 < 0.0 && y0 < 0.0 && x1 > 20.0 && y1 > 20.0);

    let (x0, _, x1, _) = inside(&deformed(0, 8.0, 1.0, 0.0, 4.0));
    assert!(x0 <= 2.0 && x1 >= 98.0, "grown by the amount");
    let (x0, _, x1, _) = inside(&deformed(1, 50.0, 1.0, 0.0, 4.0));
    assert!(x0 <= -10.0 && x1 >= 110.0, "grown by the scale");

    // Twisted and then deformed, the deform grows the twist's box.
    let mut both = twisted(90.0, 30.0, (50.0, 50.0));
    both.deform = deformed(0, 8.0, 1.0, 0.0, 4.0).deform;
    inside(&both);
}

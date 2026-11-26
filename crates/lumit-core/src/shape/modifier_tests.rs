use super::bend::smoothed;
use super::geometry::dist;
use super::modifier::{self, Deform, ModifierKind, ShapeEntry, Smooth, Twist};
use super::raster::{Art, Pressed, TAPER_TIP};
use super::tests::{alpha_at, entries, item, points_of, square, with};
use super::*;
use crate::mask::Mask;

fn circle() -> BezierPath {
    Mask::ellipse(50.0, 50.0, 40.0, 40.0).path
}

fn twist(angle: f64, rotation: f64, axis: f64, centre: (f64, f64)) -> ModifierKind {
    ModifierKind::Twist(Twist {
        angle: Property::fixed(angle),
        rotation: Property::fixed(rotation),
        axis: Property::fixed(axis),
        centre_x: Property::fixed(centre.0),
        centre_y: Property::fixed(centre.1),
        extra: serde_json::Map::new(),
    })
}

fn twisted(angle: f64, axis: f64, centre: (f64, f64)) -> Vec<ShapeEntry> {
    with(item(circle()), vec![twist(angle, 0.0, axis, centre)])
}

fn drawn(contents: &[ShapeEntry]) -> Vec<u8> {
    rasterise_contents(contents, 100, 100, 0.0, 0.0, 100.0, 100.0, 0.0)
}

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
        points_of(
            &with(
                item(circle()),
                vec![twist(angle, rotation, 30.0, (20.0, 70.0))],
            ),
            0.0,
        )
        .unwrap()
    };
    same(&flatten_fine(&circle()), &turned(0.0, 0.0));
    same(&turned(180.0, 0.0), &turned(180.0, 360.0));

    let eight = drawn(&twisted(180.0, 0.0, (50.0, 50.0)));
    assert_eq!(alpha_at(&eight, 100, 50, 25), 0, "the eight crosses here");
    assert_eq!(alpha_at(&eight, 100, 30, 50), 255, "a lobe either side");
    assert_eq!(alpha_at(&eight, 100, 70, 50), 255);

    let upright = drawn(&twisted(180.0, 90.0, (50.0, 50.0)));
    assert_eq!(alpha_at(&upright, 100, 25, 50), 0);
    assert_eq!(alpha_at(&upright, 100, 50, 30), 255);
}

fn deform(mode: u32, amount: f64, seed: f64, evolution: f64, detail: f64) -> ModifierKind {
    ModifierKind::Deform(Deform {
        mode,
        amount: Property::fixed(amount),
        scale: Property::fixed(10.0),
        evolution: Property::fixed(evolution),
        seed: Property::fixed(seed),
        detail: Property::fixed(detail),
        extra: serde_json::Map::new(),
    })
}

fn deformed(mode: u32, amount: f64, seed: f64, evolution: f64, detail: f64) -> Vec<ShapeEntry> {
    with(
        item(circle()),
        vec![deform(mode, amount, seed, evolution, detail)],
    )
}

fn bends(points: &[(f64, f64)]) -> usize {
    let r: Vec<f64> = points.iter().map(|&p| dist(p, (50.0, 50.0))).collect();
    r.windows(3)
        .filter(|w| (w[1] - w[0]) * (w[2] - w[1]) < 0.0)
        .count()
}

#[test]
fn a_deform_moves_the_outline_no_further_than_its_amount_and_repeats_itself() {
    let plain = drawn(&entries(vec![item(circle())]));
    let none = deformed(0, 0.0, 1.0, 0.0, 4.0);
    assert!(points_of(&none, 0.0).is_none(), "nothing to bend");
    assert_eq!(drawn(&none), plain);

    let pushed = points_of(&deformed(0, 8.0, 1.0, 0.0, 4.0), 0.0).unwrap();
    let (lo, hi) = radii(&pushed);
    assert!(lo < 39.0 && hi > 41.0, "moved both ways: {lo}..{hi}");
    assert!(lo > 31.9 && hi < 48.1, "and within the amount: {lo}..{hi}");

    let scaled = points_of(&deformed(1, 50.0, 1.0, 0.0, 4.0), 0.0).unwrap();
    let (lo, hi) = radii(&scaled);
    assert!(lo < 39.0 && hi > 41.0, "moved both ways: {lo}..{hi}");
    assert!(lo > 19.9 && hi < 60.1, "and within the amount: {lo}..{hi}");

    let again = points_of(&deformed(0, 8.0, 1.0, 0.0, 4.0), 0.0).unwrap();
    assert_eq!(pushed, again);
    assert_ne!(
        pushed,
        points_of(&deformed(0, 8.0, 2.0, 0.0, 4.0), 0.0).unwrap()
    );
    assert_ne!(
        pushed,
        points_of(&deformed(0, 8.0, 1.0, 90.0, 4.0), 0.0).unwrap()
    );

    let broad = points_of(&deformed(0, 8.0, 1.0, 0.0, 0.1), 0.0).unwrap();
    assert!(
        bends(&broad) * 4 < bends(&pushed),
        "{} against {}",
        bends(&broad),
        bends(&pushed)
    );
}

#[test]
fn a_smooth_rounds_the_path_inside_its_points_and_an_open_one_keeps_its_ends() {
    let smooth = |amount: f64| {
        ModifierKind::Smooth(Smooth {
            amount: Property::fixed(amount),
            extra: serde_json::Map::new(),
        })
    };
    let still = with(item(square(0.0, 0.0, 40.0)), vec![smooth(0.0)]);
    assert!(points_of(&still, 0.0).is_none(), "nothing to round");

    let round = points_of(&with(item(square(0.0, 0.0, 40.0)), vec![smooth(10.0)]), 0.0).unwrap();
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

    let mut start_only = taper(50.0, 50.0, 0);
    start_only.end = Property::zero();
    let w = width(start_only);
    assert!(w[0] <= TAPER_TIP && (w[w.len() - 1] - 1.0).abs() < 1e-9);

    let w = width(taper(80.0, 80.0, 0));
    let widest = w.iter().copied().fold(0.0, f64::max);
    assert!((widest - 1.0).abs() < 1e-6, "full width once: {widest}");
    assert!(w[0] <= TAPER_TIP && w[w.len() - 1] <= TAPER_TIP);
    let peak = w.iter().position(|&x| x == widest).unwrap() as f64 / w.len() as f64;
    assert!((peak - 0.5).abs() < 0.05, "in the middle: {peak}");

    let lines = |scope: u32| {
        let mut a = item(square(0.0, 0.0, 20.0));
        a.stroke = Some(colour::still(LinearColour([0.0, 1.0, 0.0, 1.0])));
        a.stroke_width = Property::fixed(4.0);
        a.taper = Some(taper(50.0, 50.0, scope));
        let mut b = item(square(40.0, 0.0, 20.0));
        b.combine = 1;
        let contents = entries(vec![a, b]);
        let items = modifier::lowered(&contents);
        let art = Art::at(&items, 0..2, 0.0).unwrap();
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

#[test]
fn the_box_holds_a_twisted_or_deformed_shape() {
    let inside = |contents: &[ShapeEntry]| {
        let art = modifier::lowered(contents);
        let (x0, y0, x1, y1) = art[0].item.bounds(&art[0].mods, 0.0).unwrap();
        let points = points_of(contents, 0.0).unwrap();
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
    let slant = with(
        item(square(0.0, 0.0, 20.0)),
        vec![twist(30.0, 0.0, 45.0, (10.0, 10.0))],
    );
    let (x0, y0, x1, y1) = inside(&slant);
    assert!(x0 < 0.0 && y0 < 0.0 && x1 > 20.0 && y1 > 20.0);

    let (x0, _, x1, _) = inside(&deformed(0, 8.0, 1.0, 0.0, 4.0));
    assert!(x0 <= 2.0 && x1 >= 98.0, "grown by the amount");
    let (x0, _, x1, _) = inside(&deformed(1, 50.0, 1.0, 0.0, 4.0));
    assert!(x0 <= -10.0 && x1 >= 110.0, "grown by the scale");

    let both = with(
        item(circle()),
        vec![
            twist(90.0, 0.0, 30.0, (50.0, 50.0)),
            deform(0, 8.0, 1.0, 0.0, 4.0),
        ],
    );
    inside(&both);
}

/// FNV-1a over `bytes`: a stable name for a buffer, so a picture can be
/// pinned without keeping the picture.
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// A file from before modifiers were entries still draws the same pixels.
///
/// The fixture is a shape layer written by 0.9's `ShapeItem`, with every
/// modifier field in use: a trim, dashes, an offset, a repeater with the
/// whole step and the fade, a twist, a smooth, both deforms, a taper, a
/// combined run wearing its first item's modifiers, keyed values, a morph,
/// and a plain item between them that no modifier may reach. The box, the
/// picture and the outline an effect walks are pinned at two times.
#[test]
fn an_older_files_modifiers_draw_as_they_did() {
    let kind: crate::model::LayerKind =
        serde_json::from_str(include_str!("fixtures/old-modifiers.json")).expect("reads");
    let crate::model::LayerKind::Shape { contents } = kind else {
        panic!("a shape layer");
    };
    let context = std::sync::Arc::new(crate::expression::ExpressionContext {
        document: std::sync::Arc::new(crate::model::Document::new()),
        comp: None,
        layer: None,
        comp_time: 0.0,
        current_depth: 0,
        inputs: None,
    });
    type Pinned = (f64, (f64, f64, f64, f64), u64, u64);
    let expected: [Pinned; 2] = [
        (
            0.0,
            (-1.1630491652392632, 3.5, 194.0, 164.5),
            17382974129057078523,
            5266357332587173629,
        ),
        (
            1.0,
            (-1.1630491652392632, 3.5, 194.0, 163.75),
            8980961866031931577,
            13633090624302635690,
        ),
    ];
    for (t, bounds, picture, outline) in expected {
        let settled = contents_at(&contents, t, &context);
        let (x0, y0, x1, y1) = contents_bounds(&settled, t).expect("a box");
        let (w, h) = ((x1 - x0).round() as u32, (y1 - y0).round() as u32);
        let rgba = rasterise_contents(&settled, w, h, x0, y0, x1, y1, t);
        let line = outline_at(&contents, t);
        let mut walked = Vec::new();
        for p in &line.points {
            walked.extend_from_slice(&p[0].to_le_bytes());
            walked.extend_from_slice(&p[1].to_le_bytes());
        }
        for &c in &line.corners {
            walked.extend_from_slice(&(c as u32).to_le_bytes());
        }
        assert_eq!(
            ((x0, y0, x1, y1), fnv(&rgba), fnv(&walked)),
            (bounds, picture, outline),
            "at {t} s"
        );
    }
}

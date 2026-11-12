//! Sort, Math, Contour and Intersection points.

use super::*;

/// **Sort distribution puts the stream in a new order and keeps the ids**: across,
/// reversed, and a shuffle that is the same for one seed and another for
/// another.
#[test]
fn sort_points_orders_the_stream_and_keeps_the_ids() {
    use effects::sort_points::SortPoints;
    let mut stream = points::PointsStream::default();
    for (i, x) in [30.0f32, 10.0, 20.0, 40.0].into_iter().enumerate() {
        stream.position.push([x, 0.0, 0.0]);
        stream.size.push(1.0);
        stream.id.push(i as u64);
    }
    let across = SortPoints {
        sort_by: 1,
        ..SortPoints::read(Params::EMPTY)
    };
    let out = across.apply(&stream);
    let xs: Vec<f32> = out.position.iter().map(|p| p[0]).collect();
    assert_eq!(xs, [10.0, 20.0, 30.0, 40.0]);
    assert_eq!(out.id, [1, 2, 0, 3], "each point keeps its id");
    let back = SortPoints {
        reverse: true,
        ..across
    }
    .apply(&stream);
    assert_eq!(back.id, [3, 0, 2, 1]);
    let shuffle = |seed| {
        SortPoints {
            sort_by: 0,
            seed,
            ..SortPoints::read(Params::EMPTY)
        }
        .apply(&stream)
        .id
    };
    assert_eq!(shuffle(1), shuffle(1), "one seed, one shuffle");
    let mut all = shuffle(1);
    all.sort_unstable();
    assert_eq!(all, [0, 1, 2, 3], "every point still there");
}

/// **Math points lays out its formula**: a Linear wave across from one side
/// to the other, and a rose of four petals within its radius, through the
/// middle.
#[test]
fn math_points_lays_out_waves_and_a_rose() {
    use effects::math_points::MathPoints;
    let base = MathPoints {
        count: 5,
        position_x: 0.0,
        position_y: 0.0,
        ..MathPoints::read(Params::EMPTY)
    };
    let waves = MathPoints {
        x_wave: 0,
        x_amplitude: 100.0,
        y_amplitude: 0.0,
        ..base
    }
    .stream(points::Projection::FLAT);
    let xs: Vec<f32> = waves.position.iter().map(|p| p[0]).collect();
    assert_eq!(xs, [-100.0, -50.0, 0.0, 50.0, 100.0]);
    let rose = MathPoints {
        shape: 1,
        count: 64,
        radius: 100.0,
        petals: 2.0,
        denominator: 1.0,
        ..base
    }
    .stream(points::Projection::FLAT);
    assert_eq!(rose.len(), 64);
    assert!(rose.position.iter().all(|p| p[0].hypot(p[1]) <= 100.001));
    assert!(
        rose.position.iter().any(|p| p[0].hypot(p[1]) < 1.0),
        "the petals meet in the middle"
    );
    // A Denominator keyed through 1 does not jump there: five petals over 1
    // lie where five over a hair more do.
    let five = |denominator: f32| {
        MathPoints {
            shape: 1,
            count: 64,
            radius: 100.0,
            petals: 5.0,
            denominator,
            ..base
        }
        .stream(points::Projection::FLAT)
    };
    let (on, near) = (five(1.0), five(1.0001));
    assert!(on
        .position
        .iter()
        .zip(&near.position)
        .all(|(a, b)| (a[0] - b[0]).hypot(a[1] - b[1]) < 0.5));
}

/// **Contour points puts one point on each piece**: two squares are two
/// points, each on its square's middle, and Size from area is the circle of
/// the square's area.
#[test]
fn contour_points_puts_a_point_on_each_piece() {
    use effects::contour_points::ContourPoints;
    let mut path = square_path(0.0, 0.0, 10.0);
    path.append(&square_path(100.0, 0.0, 20.0));
    let points = ContourPoints {
        size_from_area: true,
        ..ContourPoints::read(Params::EMPTY)
    }
    .stream(&path, points::Projection::FLAT);
    assert_eq!(points.len(), 2);
    let [x, y, _] = points.position[0];
    assert!((x - 5.0).abs() < 1e-3 && (y - 5.0).abs() < 1e-3, "{x} {y}");
    let [x, y, _] = points.position[1];
    assert!(
        (x - 110.0).abs() < 1e-3 && (y - 10.0).abs() < 1e-3,
        "{x} {y}"
    );
    let circle = 2.0 * (400.0 / std::f32::consts::PI).sqrt();
    assert!((points.size[1] - circle).abs() < 1e-2);
}

/// **Intersection points finds where paths cross, once each**: two squares
/// overlapping by a corner cross twice; touching neighbours on one square
/// are not crossings.
#[test]
fn intersection_points_finds_each_crossing_once() {
    use effects::intersection_points::crossings;
    let one = square_path(0.0, 0.0, 10.0);
    assert!(crossings(&one).is_empty(), "a square does not cross itself");
    let mut two = one.clone();
    two.append(&square_path(5.0, 5.0, 10.0));
    let mut found: Vec<[f32; 2]> = crossings(&two).into_iter().map(|(p, _)| p).collect();
    found.sort_by(|a, b| a[0].total_cmp(&b[0]));
    assert_eq!(found.len(), 2, "{found:?}");
    assert!((found[0][0] - 5.0).abs() < 1e-3 && (found[0][1] - 10.0).abs() < 1e-3);
    assert!((found[1][0] - 10.0).abs() < 1e-3 && (found[1][1] - 5.0).abs() < 1e-3);
}

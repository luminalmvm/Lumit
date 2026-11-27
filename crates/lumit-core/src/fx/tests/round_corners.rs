//! Round corners on the paths a cut has nicked.

use super::*;

/// **A corner a cut has nicked rounds by the whole radius**: a side of 3 px
/// across a corner is closed back into the corner first, so the fillet is not
/// held to half of it. The end of a thin strip, whose sides run parallel, and
/// the tip of a narrow wedge stay.
#[test]
fn round_corners_closes_a_nicked_corner() {
    use effects::clone_deformer::fillet::{filleted, without_nicks};
    let nicked = vec![
        [0.0, 0.0],
        [100.0, 0.0],
        [100.0, 97.0],
        [97.0, 100.0],
        [0.0, 100.0],
    ];
    let closed = without_nicks(&nicked, 20.0);
    assert_eq!(closed.len(), 4);
    assert!(closed
        .iter()
        .any(|p| (p[0] - 100.0).abs() < 1e-3 && (p[1] - 100.0).abs() < 1e-3));
    let round = filleted(&nicked, 20.0);
    // The arc starts 20 px from the corner on both sides, as on a clean one.
    assert!(round
        .iter()
        .any(|p| (p[1] - 100.0).abs() < 1e-3 && (p[0] - 80.0).abs() < 0.5));
    assert!(round
        .iter()
        .any(|p| (p[0] - 100.0).abs() < 1e-3 && (p[1] - 80.0).abs() < 0.5));
    // A strip's end and a wedge's tip are sides of their own.
    let strip = vec![[0.0, 0.0], [100.0, 0.0], [100.0, 4.0], [0.0, 4.0]];
    assert_eq!(without_nicks(&strip, 5.0).len(), 4);
    let wedge = [[0.0, 0.0], [200.0, -1.5], [200.0, 1.5]];
    let wedge = vec![wedge[0], wedge[1], wedge[2], [0.0, 3.0]];
    assert_eq!(without_nicks(&wedge, 5.0).len(), 4);
    // A point a cut left on a straight side goes, so the corner beside it
    // rounds by the whole radius.
    let grazed = vec![
        [0.0, 0.0],
        [100.0, 0.0],
        [100.0, 84.0],
        [100.1, 100.0],
        [0.0, 100.0],
    ];
    assert_eq!(without_nicks(&grazed, 20.0).len(), 4);
    let round = filleted(&grazed, 20.0);
    assert!(round
        .iter()
        .any(|p| (p[0] - 100.0).abs() < 0.2 && (p[1] - 80.0).abs() < 0.6));
    // A thin ray's end keeps its corners, though they sit within a pixel of
    // the line through their neighbours.
    let ray = [[0.0, 0.0], [200.0, -0.8], [200.0, 0.8]];
    assert_eq!(
        without_nicks(&[ray[0], ray[1], ray[2], [0.0, 1.2]], 20.0).len(),
        4
    );
    // A ray whose long sides are drawn in many nearly straight pieces keeps
    // its length.
    let ray = vec![
        [148.0, 206.0],
        [148.0, 148.0],
        [168.0, 148.0],
        [203.0, 185.0],
        [281.0, 267.0],
        [367.0, 359.0],
        [460.0, 457.0],
        [475.0, 473.0],
        [458.0, 459.0],
        [353.0, 374.0],
        [255.0, 294.0],
        [168.0, 223.0],
    ];
    // Its tip, a stub of three points, stays blunt, and the rounded ray still
    // reaches it.
    let kept = without_nicks(&ray, 37.0);
    for p in [[460.0, 457.0], [475.0, 473.0], [458.0, 459.0]] {
        assert!(kept.contains(&p), "{kept:?}");
    }
    let round = filleted(&ray, 37.0);
    assert!(
        round
            .iter()
            .any(|p| (p[0] - 475.0).hypot(p[1] - 473.0) < 12.0),
        "{round:?}"
    );
    // A tip whose sides would meet close by, but at 55°, stays a blunt tip.
    let tip = vec![[-17.74, -11.24], [0.0, -2.0], [0.0, 2.0], [-17.74, 11.24]];
    assert_eq!(without_nicks(&tip, 10.0).len(), 4);
    // A point a cut left alone on a long straight side, between two real
    // corners, goes too; the points of a curve, each barely turning, stay.
    let lone = vec![
        [398.13803, 457.28122],
        [478.7039, 457.28125],
        [458.489, 492.5676],
        [438.42093, 527.5977],
    ];
    assert_eq!(without_nicks(&lone, 37.0).len(), 3);
    let curve: Vec<[f32; 2]> = (0..90)
        .map(|k| {
            let t = k as f32 * std::f32::consts::TAU / 90.0;
            [200.0 * t.cos(), 200.0 * t.sin()]
        })
        .collect();
    assert_eq!(without_nicks(&curve, 37.0).len(), 90);
}

/// Round corners' Radius is how far along each side the rounding starts, as
/// Cavalry's Bevel: a 120° corner's arc starts 20 px from it on both sides.
#[test]
fn round_corners_radius_is_along_the_sides() {
    use effects::clone_deformer::fillet::filleted;
    let h = 100.0 * 3f32.sqrt() / 2.0;
    let hexagon: Vec<[f32; 2]> = (0..6)
        .map(|k| {
            let t = k as f32 * std::f32::consts::FRAC_PI_3;
            [100.0 * t.cos(), 100.0 * t.sin()]
        })
        .collect();
    let round = filleted(&hexagon, 20.0);
    // Along the side from (100, 0) to (50, h), 20 px from the first corner.
    let start = [100.0 - 0.2 * 50.0, 0.2 * h];
    assert!(
        round
            .iter()
            .any(|p| (p[0] - start[0]).hypot(p[1] - start[1]) < 0.05),
        "{round:?}"
    );
}

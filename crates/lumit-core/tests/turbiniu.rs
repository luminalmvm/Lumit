use lumit_core::fx::{
    effects::turbiniu::Turbiniu, params::CurvePoints, params::Params, registry::EffectMetadata,
};
use lumit_core::mask::MaskPolyline;

fn path(points: &[[f32; 2]], closed: bool) -> MaskPolyline {
    let mut arc = vec![0.0];
    let mut distance = 0.0;
    for pair in points.windows(2) {
        distance += (pair[1][0] - pair[0][0]).hypot(pair[1][1] - pair[0][1]);
        arc.push(distance);
    }
    MaskPolyline {
        points: points.to_vec(),
        arc,
        closed,
        ..Default::default()
    }
}

fn controls() -> Turbiniu {
    let constant = CurvePoints::sanitised(&[[0.0, 1.0], [1.0, 1.0]]);
    Turbiniu {
        iterations: 1,
        curve_segments: 12,
        mask_1_thickness: 8.0,
        mask_2_thickness: 8.0,
        mask_1_thickness_ramp: constant,
        mask_2_thickness_ramp: constant,
        mask_1_opacity_ramp: constant,
        mask_2_opacity_ramp: constant,
        ..Turbiniu::read(Params::EMPTY)
    }
}

#[test]
fn a_shifted_full_turn_matches_the_unshifted_stroke() {
    let guide = path(
        &[
            [20.0, 20.0],
            [100.0, 20.0],
            [100.0, 100.0],
            [20.0, 100.0],
            [20.0, 20.0],
        ],
        true,
    );
    let unshifted = Turbiniu {
        stroke_loop: true,
        cap_begin: 1,
        cap_end: 2,
        ..controls()
    };
    let shifted = Turbiniu {
        stroke_offset: 33.0,
        ..unshifted
    };
    let base = unshifted.geometry(&guide, &guide, 1.0, [128, 128]);
    assert_eq!(base.len(), 12 * 18);
    assert_eq!(base, shifted.geometry(&guide, &guide, 1.0, [128, 128]));
}

#[test]
fn markers_align_to_the_displaced_path() {
    let guide = path(&[[20.0, 40.0], [120.0, 40.0]], false);
    let fx = Turbiniu {
        primitives: 1,
        point_segments: 4,
        rotation_along_lines: 100.0,
        mask_1_sine_1_amplitude: 10.0,
        ..controls()
    };
    let vertices = fx.geometry(&guide, &guide, 1.0, [160, 100]);
    assert!(vertices[1][0] > vertices[0][0]);
    assert!(vertices[1][1] < vertices[0][1] - 0.5);
}

#[test]
fn only_coincident_marker_endpoints_are_deduplicated() {
    let guide = path(
        &[
            [20.0, 20.0],
            [100.0, 20.0],
            [100.0, 100.0],
            [20.0, 100.0],
            [20.0, 20.0],
        ],
        true,
    );
    let fx = Turbiniu {
        primitives: 1,
        point_segments: 4,
        curve_segments: 4,
        ..controls()
    };
    let vertices_per_marker = 4 * 9;
    assert_eq!(
        fx.geometry(&guide, &guide, 1.0, [128, 128]).len(),
        4 * vertices_per_marker
    );
    let warped = Turbiniu {
        mask_1_sine_1_amplitude: 10.0,
        mask_1_sine_1_frequency: 0.25,
        ..fx
    };
    let vertices = warped.geometry(&guide, &guide, 1.0, [128, 128]);
    assert_eq!(vertices.len(), 5 * vertices_per_marker);
    assert_eq!(&vertices[4 * vertices_per_marker][..2], &[10.0, 20.0]);
}

#[test]
fn the_marker_budget_reaches_both_guides() {
    let first = path(&[[20.0, 10.0], [120.0, 10.0]], false);
    let last = path(&[[20.0, 110.0], [120.0, 110.0]], false);
    let fx = Turbiniu {
        primitives: 1,
        iterations: 10_000,
        curve_segments: 10_000,
        point_segments: 128,
        ..controls()
    };
    let vertices = fx.geometry(&first, &last, 1.0, [160, 128]);
    let vertices_per_marker = 8 * 9;
    assert_eq!(vertices.len(), 10_000 * 2 * vertices_per_marker);
    assert_eq!(vertices[0][1], 10.0);
    assert_eq!(vertices[vertices.len() - vertices_per_marker][1], 110.0);
}

/// The middle of every marker drawn, from the centre vertex its fan starts on.
fn marker_centres(vertices: &[[f32; 6]], sides: usize) -> Vec<[f32; 2]> {
    vertices
        .chunks(9 * sides)
        .map(|marker| [marker[0][0], marker[0][1]])
        .collect()
}

#[test]
fn trimming_markers_keeps_the_ones_that_stay() {
    let guide = path(&[[0.0, 50.0], [200.0, 50.0]], false);
    let fx = Turbiniu {
        primitives: 1,
        point_segments: 4,
        curve_segments: 40,
        visible_points: 50,
        ..controls()
    };
    let whole = marker_centres(&fx.geometry(&guide, &guide, 1.0, [200, 100]), 4);
    let trimmed = Turbiniu {
        stroke_begin: 25.0,
        ..fx
    };
    let rest = marker_centres(&trimmed.geometry(&guide, &guide, 1.0, [200, 100]), 4);
    let kept: Vec<_> = whole
        .iter()
        .filter(|c| c[0] >= 50.0 - 1e-3)
        .copied()
        .collect();
    assert!(
        !kept.is_empty() && kept.len() < 31,
        "about half show: {}",
        kept.len()
    );
    assert_eq!(rest, kept);
}

#[test]
fn travel_carries_the_markers_along() {
    let guide = path(&[[0.0, 50.0], [200.0, 50.0]], false);
    let fx = Turbiniu {
        primitives: 1,
        point_segments: 4,
        curve_segments: 40,
        visible_points: 50,
        stroke_begin: 20.0,
        stroke_end: 60.0,
        ..controls()
    };
    let here = marker_centres(&fx.geometry(&guide, &guide, 1.0, [200, 100]), 4);
    let moved = Turbiniu {
        stroke_offset: 25.0,
        ..fx
    };
    let there = marker_centres(&moved.geometry(&guide, &guide, 1.0, [200, 100]), 4);
    let shifted: Vec<_> = here.iter().map(|c| [c[0] + 50.0, c[1]]).collect();
    assert_eq!(there.len(), shifted.len());
    for (a, b) in there.iter().zip(&shifted) {
        assert!((a[0] - b[0]).abs() < 1e-3, "{a:?} vs {b:?}");
    }
}

#[test]
fn a_solid_arrow_on_an_unseen_stroke_is_unseen() {
    let guide = path(&[[20.0, 50.0], [180.0, 50.0]], false);
    let fx = Turbiniu {
        mask_1_thickness: 0.0,
        mask_2_thickness: 0.0,
        cap_end: 1,
        arrow_length_end: 20.0,
        ..controls()
    };
    let vertices = fx.geometry(&guide, &guide, 1.0, [200, 100]);
    assert!(vertices.iter().all(|v| v[5] == 0.0));
}

#[test]
fn a_solid_arrow_sits_on_the_stroke_end() {
    let guide = path(&[[20.0, 50.0], [180.0, 50.0]], false);
    let fx = Turbiniu {
        cap_end: 1,
        arrow_length_end: 20.0,
        arrow_angle_end: 45.0,
        ..controls()
    };
    let vertices = fx.geometry(&guide, &guide, 1.0, [200, 100]);
    let head = &vertices[vertices.len() - 3..];
    assert!((head[1][0] - 180.0).abs() < 1e-3 && (head[2][0] - 180.0).abs() < 1e-3);
    assert!((head[0][0] - (180.0 + 20.0 * 45f32.to_radians().cos())).abs() < 1e-3);
}

#[test]
fn palette_jitter_moves_colour_and_not_width() {
    let first = path(&[[20.0, 20.0], [180.0, 20.0]], false);
    let last = path(&[[20.0, 80.0], [180.0, 80.0]], false);
    let fx = Turbiniu {
        iterations: 6,
        mask_1_thickness: 2.0,
        mask_2_thickness: 12.0,
        ..controls()
    };
    let plain = fx.geometry(&first, &last, 1.0, [200, 100]);
    let jittered = Turbiniu {
        gradient_random: 100.0,
        ..fx
    }
    .geometry(&first, &last, 1.0, [200, 100]);
    assert_eq!(plain.len(), jittered.len());
    for (a, b) in plain.iter().zip(&jittered) {
        assert_eq!(&a[..2], &b[..2]);
        assert_eq!(a[5], b[5]);
    }
}

#[test]
fn the_coverage_wave_does_not_follow_path_resolution() {
    let guide = path(&[[0.0, 50.0], [200.0, 50.0]], false);
    let coverage_at_quarters = |segments| {
        let fx = Turbiniu {
            curve_segments: segments,
            alpha_noise_brightness: 0.5,
            alpha_sinus_amplitude: 0.5,
            alpha_sinus_frequency: 3.0,
            ..controls()
        };
        let vertices = fx.geometry(&guide, &guide, 1.0, [200, 100]);
        // The strip's inner vertices carry the coverage; read it at x = 50,
        // 100, 150.
        [50.0f32, 100.0, 150.0].map(|x| {
            vertices
                .iter()
                .filter(|v| (v[0] - x).abs() < 1e-3 && v[5] > 0.0)
                .map(|v| v[5])
                .fold(0.0, f32::max)
        })
    };
    let coarse = coverage_at_quarters(8);
    let fine = coverage_at_quarters(40);
    for (a, b) in coarse.iter().zip(&fine) {
        assert!((a - b).abs() < 1e-4, "{coarse:?} vs {fine:?}");
    }
}

#[test]
fn a_guide_wave_pushes_by_its_depth_in_pixels() {
    let guide = path(&[[0.0, 50.0], [200.0, 50.0]], false);
    let fx = Turbiniu {
        curve_segments: 4,
        mask_1_sine_1_amplitude: 30.0,
        mask_2_sine_1_amplitude: 30.0,
        ..controls()
    };
    // Depth arrives in raster pixels (a px@comp control), so at half scale a
    // depth of 30 still pushes 30: the stroke's middle a quarter along, at
    // x = 25, sits 30 above the halved path's y = 25.
    let vertices = fx.geometry(&guide, &guide, 0.5, [100, 50]);
    let at_quarter: Vec<f32> = vertices
        .iter()
        .filter(|v| (v[0] - 25.0).abs() < 1e-3)
        .map(|v| v[1])
        .collect();
    let middle = at_quarter.iter().sum::<f32>() / at_quarter.len() as f32;
    assert!((middle - (25.0 - 30.0)).abs() < 1e-3, "{middle}");
}

/// Distinct vertex positions, to compare outlines.
fn outline(vertices: &[[f32; 6]]) -> Vec<[u32; 2]> {
    let mut at: Vec<[u32; 2]> = vertices
        .iter()
        .map(|v| [v[0].to_bits(), v[1].to_bits()])
        .collect();
    at.sort_unstable();
    at.dedup();
    at
}

#[test]
fn trimming_a_stroke_keeps_its_outline() {
    let guide = path(&[[0.0, 50.0], [200.0, 50.0]], false);
    let fx = Turbiniu {
        curve_segments: 8,
        mask_1_sine_1_amplitude: 20.0,
        mask_1_sine_1_frequency: 3.0,
        mask_2_sine_1_amplitude: 20.0,
        mask_2_sine_1_frequency: 3.0,
        ..controls()
    };
    let whole = outline(&fx.geometry(&guide, &guide, 1.0, [200, 100]));
    let trimmed = outline(
        &Turbiniu {
            stroke_begin: 10.0,
            ..fx
        }
        .geometry(&guide, &guide, 1.0, [200, 100]),
    );
    // Only the new end row and the row beside it, which turns towards it,
    // are new; the rest of the outline is the untrimmed one.
    let new = trimmed
        .iter()
        .filter(|p| whole.binary_search(p).is_err())
        .count();
    assert!(new <= 8, "{new} new vertices");
}

#[test]
fn a_second_lap_draws_nothing_more() {
    let guide = path(
        &[
            [20.0, 20.0],
            [100.0, 20.0],
            [100.0, 100.0],
            [20.0, 100.0],
            [20.0, 20.0],
        ],
        true,
    );
    let once = Turbiniu {
        stroke_loop: true,
        ..controls()
    };
    let twice = Turbiniu {
        stroke_end: 200.0,
        ..once
    };
    assert_eq!(
        once.geometry(&guide, &guide, 1.0, [128, 128]),
        twice.geometry(&guide, &guide, 1.0, [128, 128])
    );
}

#[test]
fn a_negative_span_reverses_the_profile() {
    let guide = path(&[[0.0, 50.0], [200.0, 50.0]], false);
    let rising = CurvePoints::sanitised(&[[0.0, 0.0], [1.0, 1.0]]);
    let width_at = |fx: Turbiniu, x: f32| {
        fx.geometry(&guide, &guide, 1.0, [200, 100])
            .iter()
            .filter(|v| (v[0] - x).abs() < 1e-3)
            .map(|v| (v[1] - 50.0).abs())
            .fold(0.0f32, f32::max)
    };
    let fx = Turbiniu {
        mask_1_thickness: 20.0,
        mask_2_thickness: 20.0,
        mask_1_thickness_ramp: rising,
        mask_2_thickness_ramp: rising,
        mask_1_thickness_ramp_scale: -100.0,
        mask_2_thickness_ramp_scale: -100.0,
        curve_segments: 4,
        ..controls()
    };
    assert!(width_at(fx, 0.0) > width_at(fx, 200.0) + 5.0);
}

#[test]
fn markers_on_a_trim_that_starts_before_the_path() {
    let guide = path(&[[0.0, 50.0], [200.0, 50.0]], false);
    let fx = Turbiniu {
        primitives: 1,
        point_segments: 4,
        curve_segments: 20,
        stroke_begin: -50.0,
        stroke_offset: 25.0,
        stroke_loop: true,
        ..controls()
    };
    assert!(!fx.geometry(&guide, &guide, 1.0, [200, 100]).is_empty());
}

#[test]
fn a_trim_in_before_the_start_keeps_trim_out() {
    let guide = path(&[[0.0, 50.0], [200.0, 50.0]], false);
    let fx = Turbiniu {
        stroke_end: 90.0,
        ..controls()
    };
    let early = Turbiniu {
        stroke_begin: -45.0,
        ..fx
    };
    assert_eq!(
        fx.geometry(&guide, &guide, 1.0, [200, 100]),
        early.geometry(&guide, &guide, 1.0, [200, 100])
    );
}

#[test]
fn a_wrapped_marker_keeps_its_chance_on_every_lap() {
    let guide = path(&[[0.0, 50.0], [200.0, 50.0]], false);
    let fx = Turbiniu {
        primitives: 1,
        point_segments: 4,
        curve_segments: 15,
        visible_points: 40,
        stroke_begin: -16.0,
        stroke_end: 92.0,
        stroke_offset: 5.0,
        stroke_loop: true,
        ..controls()
    };
    let whole = marker_centres(&fx.geometry(&guide, &guide, 1.0, [200, 100]), 4);
    let trimmed = marker_centres(
        &Turbiniu {
            stroke_begin: -6.0,
            ..fx
        }
        .geometry(&guide, &guide, 1.0, [200, 100]),
        4,
    );
    for c in &trimmed {
        assert!(
            whole
                .iter()
                .any(|w| (w[0] - c[0]).abs() < 1e-3 && (w[1] - c[1]).abs() < 1e-3),
            "{c:?} appeared"
        );
    }
}

/// One straight stroke 400 px long, cut into 200 pieces.
fn brush() -> (MaskPolyline, Turbiniu) {
    (
        path(&[[0.0, 50.0], [400.0, 50.0]], false),
        Turbiniu {
            curve_segments: 200,
            ..controls()
        },
    )
}

/// The coverage at each cut along a stroke: the inner band's first corner,
/// 18 vertices to a piece.
fn coverages(vertices: &[[f32; 6]]) -> Vec<f32> {
    vertices.chunks(18).map(|piece| piece[6][5]).collect()
}

#[test]
fn a_dry_brush_opens_gaps_that_grow_with_its_profile() {
    let (guide, fx) = brush();
    let at = |fx: Turbiniu| coverages(&fx.geometry(&guide, &guide, 1.0, [400, 100]));
    // Nothing changes until Dry brush is raised.
    assert!(at(fx).iter().all(|&a| (a - 1.0).abs() < 1e-6));
    let flat = CurvePoints::sanitised(&[[0.0, 1.0], [1.0, 1.0]]);
    let all = at(Turbiniu {
        dry_brush: 100.0,
        dry_brush_ramp: flat,
        ..fx
    });
    assert!(all.iter().all(|&a| a < 1e-6), "fully dry leaves nothing");
    let half = at(Turbiniu {
        dry_brush: 50.0,
        dry_brush_ramp: flat,
        ..fx
    });
    assert!(half.iter().any(|&a| a > 0.9) && half.iter().any(|&a| a < 0.1));
    // The default profile dries towards the end.
    let ramped = at(Turbiniu {
        dry_brush: 100.0,
        ..fx
    });
    let mean = |s: &[f32]| s.iter().sum::<f32>() / s.len() as f32;
    let quarter = ramped.len() / 4;
    assert!(mean(&ramped[..quarter]) > mean(&ramped[ramped.len() - quarter..]) + 0.3);
}

#[test]
fn edge_roughness_eats_each_side_on_its_own() {
    let (guide, fx) = brush();
    let rough = Turbiniu {
        edge_roughness: 100.0,
        ..fx
    }
    .geometry(&guide, &guide, 1.0, [400, 100]);
    let smooth = fx.geometry(&guide, &guide, 1.0, [400, 100]);
    let width = |piece: &[[f32; 6]]| (piece[7][1] - piece[6][1]).abs();
    let middle = |piece: &[[f32; 6]]| (piece[7][1] + piece[6][1]) * 0.5;
    let full = width(&smooth[..18]);
    let pieces: Vec<_> = rough.chunks(18).collect();
    assert!(pieces.iter().all(|p| width(p) <= full + 1e-4));
    assert!(pieces.iter().any(|p| width(p) < full * 0.7));
    assert!(pieces.iter().any(|p| (middle(p) - 50.0).abs() > 0.3));
}

#[test]
fn hardness_presses_coverage_to_on_or_off() {
    let (guide, fx) = brush();
    let at = |base: f32, hardness: f32| {
        coverages(
            &Turbiniu {
                alpha_noise_brightness: base,
                coverage_hardness: hardness,
                ..fx
            }
            .geometry(&guide, &guide, 1.0, [400, 100]),
        )[10]
    };
    assert!((at(0.4, 0.0) - 0.4).abs() < 1e-5);
    assert!(at(0.4, 100.0) < 1e-5);
    assert!(at(0.6, 100.0) > 1.0 - 1e-5);
    assert!(at(0.4, 50.0) < 0.4 && at(0.6, 50.0) > 0.6);
}

#[test]
fn density_colour_follows_how_much_paint_there_is() {
    let (guide, fx) = brush();
    let colour = |base: f32| {
        let v = Turbiniu {
            alpha_noise_brightness: base,
            density_colour: 100.0,
            density_colour_thin: [1.0, 0.0, 0.0, 1.0],
            density_colour_mid: [0.0, 1.0, 0.0, 1.0],
            density_colour_dense: [0.0, 0.0, 1.0, 1.0],
            ..fx
        }
        .geometry(&guide, &guide, 1.0, [400, 100])[6 + 18 * 10];
        [v[2] / v[5], v[3] / v[5], v[4] / v[5]]
    };
    let dense = colour(1.0);
    assert!(
        dense[2] > 0.99 && dense[0] < 0.01 && dense[1] < 0.01,
        "{dense:?}"
    );
    let thin = colour(0.25);
    assert!(
        (thin[0] - 0.5).abs() < 0.01 && (thin[1] - 0.5).abs() < 0.01,
        "{thin:?}"
    );
}

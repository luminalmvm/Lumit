use super::controls::TEXTURES;
use super::design::{ColourMode, Design, Element, Kind, FORMAT};
use super::presets::BUILT_IN;
use super::raster::{draw, Frame, Light};

const W: u32 = 96;
const H: u32 = 54;

fn frame() -> Frame {
    let mut f = Frame::preview(W, H, [20.0, 14.0], W as f32 / 1920.0);
    f.lights.push(Light {
        pos: [120.0, -10.0],
        colour: [1.0, 0.6, 0.3],
        gain: 0.5,
        scale: 0.8,
    });
    f.edge_fade = 400.0;
    f.evolution_deg = 30.0;
    f.time_s = 1.5;
    f
}

fn drawn(design: &Design) -> Vec<f32> {
    let mut rgba = vec![0.0; (W * H * 4) as usize];
    draw(design, &frame(), W, H, &mut rgba);
    rgba
}

#[test]
fn every_flare_and_every_element_draws() {
    for (_, json) in BUILT_IN {
        let parsed = Design::parse(json);
        assert!(parsed.is_some(), "a built-in flare reads");
        let Some(design) = parsed else { continue };
        let out = drawn(&design);
        assert!(out.iter().all(|v| v.is_finite()), "{}", design.name);
        assert!(
            out.iter().any(|&v| v > 0.0),
            "{} draws nothing",
            design.name
        );
    }
    let made = TEXTURES.len() - 2;
    for (n, kind) in Kind::ALL.into_iter().enumerate() {
        for (m, mode) in ColourMode::ALL.into_iter().enumerate() {
            let mut e = Element::new(kind);
            e.colour.mode = mode;
            e.colour.colours = vec![[1.0, 0.5, 0.2], [0.2, 0.6, 1.0]];
            e.set("texture", (1 + (n + m) % made) as f32);
            e.set("response", (m % 3) as f32);
            for key in ["fringe", "notches", "end_width"] {
                e.set(key, 0.5);
            }
            e.set("spin", 10.0);
            e.set("seed", m as f32);
            let design = Design {
                lumit_flare: FORMAT,
                name: String::new(),
                elements: vec![e],
            };
            let out = drawn(&design);
            assert!(out.iter().all(|v| v.is_finite()), "{kind:?} {mode:?}");
            assert_eq!(
                out,
                drawn(&design),
                "{kind:?} {mode:?} drew twice differently"
            );
        }
    }
}

/// Offset moves a streak's lines across, off the light, by its pixels: the
/// brightest row is that far from the light's, and nought leaves it there.
#[test]
fn a_streaks_offset_moves_its_lines_off_the_light() {
    let (w, h) = (200u32, 100u32);
    let brightest_row = |offset: f32| {
        let mut e = Element::new(Kind::Streak);
        e.set("thickness", 1.0);
        e.set("offset", offset);
        let design = Design {
            lumit_flare: FORMAT,
            name: String::new(),
            elements: vec![e],
        };
        let f = Frame::preview(w, h, [100.0, 50.0], 1.0);
        let mut rgba = vec![0.0; (w * h * 4) as usize];
        draw(&design, &f, w, h, &mut rgba);
        let row = |y: u32| {
            (0..w)
                .map(|x| rgba[((y * w + x) * 4) as usize])
                .sum::<f32>()
        };
        (0..h)
            .max_by(|&a, &b| row(a).total_cmp(&row(b)))
            .unwrap_or(0)
    };
    let at = brightest_row(0.0);
    assert!(at.abs_diff(50) <= 1, "on the light at nought: row {at}");
    let moved = brightest_row(20.0);
    assert!(
        moved.abs_diff(at) >= 19 && moved.abs_diff(at) <= 21,
        "20 px off: row {moved}"
    );
}

/// Burn runs light too bright for its colour toward white: a bright red
/// light gains green and blue where it is brightest, keeps its red where it
/// is dim, and Burn 0 leaves it as drawn.
#[test]
fn burn_runs_a_bright_light_to_white() {
    use super::raster::burnt;
    assert_eq!(burnt([0.8, 0.05, 0.0], 1.0), [0.8, 0.05, 0.0], "dim stays");
    assert_eq!(burnt([6.0, 0.3, 0.1], 0.0), [6.0, 0.3, 0.1], "Burn 0");
    let hot = burnt([6.0, 0.3, 0.1], 1.0);
    assert!(
        hot[1] > 4.0 && hot[2] > 3.0,
        "white where brightest: {hot:?}"
    );
    // On the way it is yellow: green past 1 before blue is.
    let yellow = burnt([2.0, 0.1, 0.04], 1.0);
    assert!(
        yellow[1] >= 1.0 && yellow[2] < 0.5,
        "yellow on the way: {yellow:?}"
    );
    let warm = burnt([1.5, 0.05, 0.0], 1.0);
    assert!(
        warm[1] > warm[2] && warm[1] < 1.0,
        "orange a little past: {warm:?}"
    );
    let (w, h) = (64u32, 64u32);
    let mut e = Element::new(Kind::Glow);
    e.set("brightness", 8.0);
    let design = Design {
        lumit_flare: FORMAT,
        name: String::new(),
        elements: vec![e],
    };
    let mut f = Frame::preview(w, h, [32.0, 32.0], 1.0);
    f.lights[0].colour = [1.0, 0.05, 0.02];
    let green = |burn: f32| {
        let mut f = f.clone();
        f.burn = burn;
        let mut rgba = vec![0.0; (w * h * 4) as usize];
        draw(&design, &f, w, h, &mut rgba);
        rgba[((32 * w + 32) * 4 + 1) as usize]
    };
    assert!(green(1.0) > 10.0 * green(0.0), "the middle burns");
}

/// Defocus blurs only light past 1: a faint streak is drawn as it was, a
/// bright glow's middle spreads.
#[test]
fn defocus_softens_the_hot_middle_and_leaves_faint_lines() {
    let (w, h) = (64u32, 48u32);
    let draw_one = |kind: Kind, brightness: f32, defocus: f32| {
        let mut e = Element::new(kind);
        e.set("brightness", brightness);
        let design = Design {
            lumit_flare: FORMAT,
            name: String::new(),
            elements: vec![e],
        };
        let mut f = Frame::preview(w, h, [32.0, 24.0], 1.0);
        f.defocus = defocus;
        let mut rgba = vec![0.0; (w * h * 4) as usize];
        draw(&design, &f, w, h, &mut rgba);
        rgba
    };
    let faint = draw_one(Kind::Streak, 0.3, 0.0);
    let faint_soft = draw_one(Kind::Streak, 0.3, 8.0);
    let gap = faint
        .iter()
        .zip(&faint_soft)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    assert!(gap < 1e-5, "a faint streak stays sharp ({gap})");
    let hot = draw_one(Kind::Glow, 20.0, 0.0);
    let hot_soft = draw_one(Kind::Glow, 20.0, 8.0);
    assert_ne!(hot, hot_soft, "the hot middle spreads");
}

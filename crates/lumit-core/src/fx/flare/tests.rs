use super::controls::TEXTURES;
use super::design::{ColourMode, Design, Element, Kind, FORMAT};
use super::presets::BUILT_IN;
use super::raster::{draw, Frame, Light};

const W: u32 = 96;
const H: u32 = 54;

/// A small frame with one light in the picture and one off it, moving and
/// turning, so every road through the drawing is taken.
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

/// Every built-in flare reads and draws, and every kind of element draws
/// finite light in every colour, response and made texture, the same twice.
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
    // The made textures, without None and Layer at the ends.
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

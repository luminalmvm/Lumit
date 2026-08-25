//! Visual proof for K-408's three consumers (docs/08 §3.78 Scribble, §3.79
//! Stroke, §3.76 Vegas' Mask/Path source): render one frame through each of
//! them on a **real masked layer**, so the kernels can be judged by eye rather
//! than asserted.
//!
//! # In plain terms
//!
//! The oracle tests prove the graphics card and the CPU agree, and the geometry
//! tests prove the strokes land where the arithmetic says. Neither can prove the
//! picture is *right*: an effect that drew a single line in the corner would
//! satisfy both. Three things here have to be looked at, and each has a
//! distinctive look it either reads as or does not:
//!
//! 1. **Scribble** must be *pencil shading inside the shape* — parallel strokes
//!    at the angle asked for, wavering rather than ruler-straight, running a
//!    little past the edge, and staying out of the notch in the notched mask. A
//!    flat fill, a comb of dead-straight lines, or shading that spills across
//!    the whole frame is the failure.
//! 2. **Stroke** must be a *brush walking the mask's own line* — following the
//!    ellipse and the squiggle rather than their filled shapes, stopping where
//!    Start and End say, and breaking into separate dots at wide Spacing. A
//!    filled shape, or an outline that ignores the trim, is the failure.
//! 3. **Vegas on Mask/Path** must be *dashes evenly spaced round the mask*, and
//!    that is the thing to look at hardest: evenly spaced **all the way round**,
//!    including where the curve is tightest. Dashes that bunch or stretch on the
//!    curve mean the arc length is not being measured, which is exactly what
//!    §3.76's contour half cannot do and this half exists to do.
//!
//! Unlike the other proofs this one **wants no footage**: the mask is the
//! subject, so the picture underneath is a card drawn here. It still writes
//! files, so it is ignored by default. Run with:
//!
//! ```text
//! LUMIT_PATH_PROOF_OUT="C:/tmp/lumit-shots" \
//!   cargo test -p lumit-render --release --test path_proof -- --ignored --nocapture
//! ```
//!
//! Output is raw RGBA8 (`<name>.<w>x<h>.raw`), for the reason `blur_proof.rs`
//! gives: nothing in the workspace encodes PNG, and a throwaway encoder written
//! for a diagnostic is exactly the code that should not exist. The runner
//! converts them.

use lumit_core::fx::cpu;
use lumit_core::fx::effects::{scribble::Scribble, stroke::Stroke, vegas::Vegas};
use lumit_core::fx::{EffectMetadata, Params};
use lumit_core::mask::{flatten_path, BezierPath, Mask, MaskPolyline, Vertex};

const W: u32 = 960;
const H: u32 = 540;

fn to_srgb(lin: &[f32]) -> Vec<u8> {
    lin.chunks_exact(4)
        .flat_map(|p| {
            [
                lumit_core::pixels::srgb_encode(p[0]),
                lumit_core::pixels::srgb_encode(p[1]),
                lumit_core::pixels::srgb_encode(p[2]),
                (p[3].clamp(0.0, 1.0) * 255.0).round() as u8,
            ]
        })
        .collect()
}

/// Flatten premultiplied RGBA over a mid-grey card, so a drawing with Composite
/// on original off reads as a shape on grey rather than as whatever the viewer
/// decides to show. `transition_proof.rs`'s helper, for its reason.
fn over_grey(lin: &[f32]) -> Vec<f32> {
    let mut out = lin.to_vec();
    for px in out.chunks_exact_mut(4) {
        let a = px[3];
        for c in &mut px[..3] {
            *c += 0.18 * (1.0 - a);
        }
        px[3] = 1.0;
    }
    out
}

/// The layer under the drawing: a soft diagonal wash with a bright bar across
/// it, so Reveal original has something recognisable to reveal and Composite on
/// original has something to sit over.
fn card() -> Vec<f32> {
    let mut img = vec![0.0f32; (W * H * 4) as usize];
    for y in 0..H {
        for x in 0..W {
            let i = ((y * W + x) * 4) as usize;
            let u = x as f32 / W as f32;
            let v = y as f32 / H as f32;
            let bar = if ((v * 9.0) as u32).is_multiple_of(2) {
                0.35
            } else {
                0.0
            };
            img[i] = 0.10 + 0.45 * u + bar;
            img[i + 1] = 0.14 + 0.30 * v + bar * 0.6;
            img[i + 2] = 0.30 + 0.25 * (1.0 - u) + bar * 0.3;
            img[i + 3] = 1.0;
        }
    }
    img
}

/// The ellipse mask, centred and generous.
fn ellipse() -> MaskPolyline {
    let m = Mask::ellipse(
        f64::from(W) * 0.30,
        f64::from(H) * 0.5,
        f64::from(W) * 0.20,
        f64::from(H) * 0.34,
    );
    lumit_core::mask::mask_path_at(std::slice::from_ref(&m), None, true, 0.0)
}

/// A closed bezier mask with a **notch** in it and some genuinely tight
/// curvature: the shape that tells you whether the pen lifts and whether the
/// dashes are measured round the curve.
fn bezier() -> MaskPolyline {
    let (cx, cy) = (f64::from(W) * 0.72, f64::from(H) * 0.5);
    let (rx, ry) = (f64::from(W) * 0.20, f64::from(H) * 0.36);
    // Eight points round an ellipse, with two of them pulled hard inwards to
    // cut a notch into the right-hand side.
    let mut vertices = Vec::new();
    for k in 0..8 {
        let a = std::f64::consts::TAU * f64::from(k) / 8.0;
        let pull = if k == 1 || k == 7 { 0.18 } else { 1.0 };
        let (px, py) = (cx + a.cos() * rx * pull, cy + a.sin() * ry * pull);
        // Tangents along the circle, scaled to the usual 4/3·tan(π/2n) for a
        // near-circular arc — enough curvature to be worth measuring.
        let t = 0.36;
        let (tx, ty) = (-a.sin() * rx * t * pull, a.cos() * ry * t * pull);
        vertices.push(Vertex {
            pos: (px, py),
            tan_in: (-tx, -ty),
            tan_out: (tx, ty),
        });
    }
    flatten_path(
        &BezierPath {
            vertices,
            closed: true,
        },
        lumit_core::mask::MASK_PATH_TOLERANCE_PX,
    )
}

#[test]
#[ignore = "harness: writes raw RGBA files"]
fn render_the_three_mask_path_effects() {
    let out_dir =
        std::env::var("LUMIT_PATH_PROOF_OUT").unwrap_or_else(|_| "C:/tmp/lumit-shots".to_owned());
    let lin = card();
    let write = |tag: &str, px: &[u8]| {
        let name = format!("{out_dir}/path.{tag}.{W}x{H}.raw");
        match std::fs::write(&name, px) {
            Ok(()) => eprintln!("wrote {name}"),
            Err(e) => eprintln!("could not write {name}: {e}"),
        }
    };
    let shot = |tag: &str, p: &cpu::PathDrawParams| {
        let mut img = lin.clone();
        cpu::path_draw(&mut img, W, H, p);
        eprintln!("{tag}: {} pieces", p.count);
        write(tag, &to_srgb(&over_grey(&img)));
    };
    write("0-source", &to_srgb(&lin));

    let (round, notched) = (ellipse(), bezier());
    eprintln!(
        "masks: ellipse {} points ({:.0} px round), bezier {} points ({:.0} px round)",
        round.points.len(),
        round.length(),
        notched.points.len(),
        notched.length()
    );

    // ---- Scribble: shading inside both masks, then a fine dense fill.
    let mut s = Scribble::read(Params::EMPTY);
    s.spacing = 14.0;
    s.stroke_width = 3.0;
    s.path_overlap = 8.0;
    s.seed = 20_260_821;
    shot("1-scribble-ellipse", &s.packed(&round, 1.0, 0.0));
    shot("2-scribble-notched", &s.packed(&notched, 1.0, 0.0));
    let mut dense = s;
    dense.spacing = 5.0;
    dense.stroke_width = 2.0;
    dense.angle = 75.0;
    dense.colour = [0.05, 0.35, 0.95, 1.0];
    shot("3-scribble-dense", &dense.packed(&notched, 1.0, 0.0));
    let mut half = s;
    half.end = 45.0;
    half.composite_on_original = false;
    shot("4-scribble-half-alone", &half.packed(&round, 1.0, 0.0));

    // ---- Stroke: the brush on both masks, trimmed, dotted, and revealing.
    let mut b = Stroke::read(Params::EMPTY);
    b.brush_size = 14.0;
    b.colour = [1.0, 0.85, 0.25, 1.0];
    shot("5-stroke-ellipse", &b.packed(&round, 1.0));
    shot("6-stroke-notched", &b.packed(&notched, 1.0));
    let mut window = b;
    window.start = 15.0;
    window.end = 65.0;
    shot("7-stroke-window", &window.packed(&notched, 1.0));
    let mut dots = b;
    dots.spacing = 260.0;
    dots.brush_size = 20.0;
    shot("8-stroke-dots", &dots.packed(&round, 1.0));
    let mut reveal = b;
    reveal.brush_size = 40.0;
    reveal.paint_style = 2;
    shot("9-stroke-reveal", &reveal.packed(&notched, 1.0));

    // ---- Vegas on Mask/Path: the dashes it could not march before K-408. Look
    // at the spacing where the notched mask curves hardest.
    let mut v = Vegas::read(Params::EMPTY);
    v.source = Vegas::SOURCE_MASK_PATH;
    v.width = 7.0;
    v.segment_length = 60.0;
    shot("10-vegas-ellipse", &v.path_packed(&round, 1.0));
    shot("11-vegas-notched", &v.path_packed(&notched, 1.0));
    let mut marched = v;
    marched.rotation = 180.0;
    shot("12-vegas-marched", &marched.path_packed(&notched, 1.0));

    // And the contour half untouched beside it, on the same card, so the two
    // sources can be compared directly — the point of the whole seam.
    let mut contour = Vegas::read(Params::EMPTY);
    contour.width = 7.0;
    contour.segment_length = 60.0;
    let mut img = lin.clone();
    cpu::vegas(&mut img, W, H, &contour.packed());
    write("13-vegas-contour", &to_srgb(&over_grey(&img)));
}

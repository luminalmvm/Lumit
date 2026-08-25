//! Visual proof for Wave 2's Distort I batch (docs/08 §3.48–§3.52): render one
//! real frame through each of the five effects at two settings, so the kernels
//! can be judged by eye rather than asserted.
//!
//! # In plain terms
//!
//! The oracle tests prove the graphics card and the CPU agree. They cannot prove
//! the picture is *right* — an effect that renders black, or noise, or the input
//! unchanged, agrees with a reference that does the same wrong thing. Five
//! things here have to be looked at:
//!
//! 1. **Corner pin** must show real perspective: the pinned picture narrows
//!    toward the far edge, and straight lines in the plate stay straight while
//!    parallel ones converge. The crossed pair is the one that matters most — the
//!    part behind the horizon must be *empty*, not a mirrored ghost.
//! 2. **Displacement map** must push the picture along the map's own shape, and
//!    the single-axis pair must move on that axis only.
//! 3. **Polar coordinates** must make a tiny planet, and the unroll must be the
//!    same picture laid flat.
//! 4. **Twirl** must spiral, hardest in the middle, with no ring at the rim.
//! 5. **Spherize** must read as glass — a swollen middle and a crowded rim — and
//!    the pinch must be its opposite rather than a smaller version of it.
//!
//! Ignored by default — it wants real footage and writes files. Run with:
//!
//! ```text
//! LUMIT_DISTORT2_PROOF_CLIPS="C:/tmp/lumit-shots/Gameplay.mp4" \
//! LUMIT_DISTORT2_PROOF_OUT="C:/tmp/lumit-shots" \
//!   cargo test -p lumit-render --release --test distort2_proof -- --ignored --nocapture
//! ```
//!
//! `LUMIT_DISTORT2_PROOF_FRAME` picks the frame (default 0). Output is raw RGBA8
//! (`<name>.<w>x<h>.raw`), for the reason `blur_proof.rs` gives: nothing in the
//! workspace encodes PNG, and a throwaway encoder written for a diagnostic is
//! exactly the code that should not exist. The runner converts them.

use lumit_core::fx::cpu;
use lumit_core::fx::effects::{
    corner_pin::CornerPin, displacement_map::DisplacementMap, polar_coordinates::PolarCoordinates,
    spherize::Spherize, twirl::Twirl,
};
use lumit_core::fx::{EffectMetadata, Params};

fn to_linear(rgba: &[u8]) -> Vec<f32> {
    rgba.chunks_exact(4)
        .flat_map(|p| {
            [
                lumit_core::pixels::srgb_decode(p[0]),
                lumit_core::pixels::srgb_decode(p[1]),
                lumit_core::pixels::srgb_decode(p[2]),
                f32::from(p[3]) / 255.0,
            ]
        })
        .collect()
}

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

#[test]
#[ignore = "harness: set LUMIT_DISTORT2_PROOF_CLIPS; writes raw RGBA files"]
fn render_the_five_distort_two_effects() {
    let Ok(clips) = std::env::var("LUMIT_DISTORT2_PROOF_CLIPS") else {
        eprintln!("set LUMIT_DISTORT2_PROOF_CLIPS to ;-separated clip paths");
        return;
    };
    let out_dir = std::env::var("LUMIT_DISTORT2_PROOF_OUT")
        .unwrap_or_else(|_| "C:/tmp/lumit-shots".to_owned());
    let frame: usize = std::env::var("LUMIT_DISTORT2_PROOF_FRAME")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    for clip in clips.split(';').filter(|s| !s.trim().is_empty()) {
        let path = std::path::Path::new(clip.trim());
        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
        let Ok(index) = lumit_media::index::build_frame_index(path) else {
            eprintln!("{stem}: could not index");
            continue;
        };
        let Ok(mut dec) = lumit_media::decode::VideoDecoder::open(path, index) else {
            eprintln!("{stem}: could not open");
            continue;
        };
        let Ok(a) = dec.frame_rgba(frame, None) else {
            eprintln!("{stem}: could not decode frame {frame}");
            continue;
        };
        let (w, h) = (a.width, a.height);
        let lin = to_linear(&a.rgba);
        let (fw, fh) = (w as f32, h as f32);

        let write = |tag: &str, px: &[u8]| {
            let name = format!("{out_dir}/{stem}.p2{frame}.{tag}.{w}x{h}.raw");
            match std::fs::write(&name, px) {
                Ok(()) => eprintln!("wrote {name}"),
                Err(e) => eprintln!("could not write {name}: {e}"),
            }
        };
        write("0-source", &a.rgba);

        // ---- Corner pin: a screen-insert keystone, and a crossed pair whose far
        // half must be empty rather than a mirrored ghost.
        let pin = |ul: [f32; 2], ur: [f32; 2], ll: [f32; 2], lr: [f32; 2], edge: u32| {
            let mut c = CornerPin::read(Params::EMPTY);
            c.upper_left_x = ul[0] * fw;
            c.upper_left_y = ul[1] * fh;
            c.upper_right_x = ur[0] * fw;
            c.upper_right_y = ur[1] * fh;
            c.lower_left_x = ll[0] * fw;
            c.lower_left_y = ll[1] * fh;
            c.lower_right_x = lr[0] * fw;
            c.lower_right_y = lr[1] * fh;
            c.edge = edge;
            c.packed()
        };
        let mut keystone = lin.clone();
        cpu::corner_pin(
            &mut keystone,
            w,
            h,
            &pin([0.28, 0.14], [0.86, 0.06], [0.24, 0.82], [0.90, 0.96], 0),
        );
        write("1-cornerpin-keystone", &to_srgb(&keystone));
        let mut crossed = lin.clone();
        cpu::corner_pin(
            &mut crossed,
            w,
            h,
            &pin([0.80, 0.20], [0.20, 0.20], [0.05, 0.95], [0.95, 0.95], 0),
        );
        write("2-cornerpin-crossed", &to_srgb(&crossed));

        // ---- Displacement map: a fractal noise map (both axes), then a
        // horizontal-only shove from a smooth ramp — the axis control, visible.
        let mut noise_map = vec![0.0f32; (w * h * 4) as usize];
        let mut fnoise = lumit_core::fx::effects::fractal_noise::FractalNoise::read(Params::EMPTY);
        fnoise.scale = fw * 0.12;
        fnoise.scale_width = fw * 0.12;
        fnoise.scale_height = fw * 0.12;
        fnoise.offset_x = fw * 0.5;
        fnoise.offset_y = fh * 0.5;
        fnoise.complexity = 4;
        fnoise.seed = 11;
        cpu::fractal_noise(&mut noise_map, w, h, &fnoise.packed());
        let dmap = |hc: u32, ha: f32, vc: u32, va: f32| {
            let mut d = DisplacementMap::read(Params::EMPTY);
            d.horizontal_channel = hc;
            d.horizontal_amount = ha;
            d.vertical_channel = vc;
            d.vertical_amount = va;
            d.packed()
        };
        let mut swirled = lin.clone();
        cpu::displacement_map(
            &mut swirled,
            w,
            h,
            &dmap(0, fw * 0.05, 0, fw * 0.05),
            &noise_map,
            false,
        );
        write("3-dispmap-fractal", &to_srgb(&swirled));

        // A vertical ramp in green: every row pushed sideways by its own height,
        // so the frame shears. If the vertical Amount leaked, it would smear.
        let mut ramp = vec![0.0f32; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let k = y as f32 / (h - 1) as f32;
                ramp[i] = k;
                ramp[i + 1] = 0.5;
                ramp[i + 2] = k;
                ramp[i + 3] = 1.0;
            }
        }
        let mut sheared = lin.clone();
        cpu::displacement_map(
            &mut sheared,
            w,
            h,
            &dmap(2, fw * 0.08, 3, 0.0),
            &ramp,
            false,
        );
        write("4-dispmap-shear-x-only", &to_srgb(&sheared));

        // ---- Polar coordinates: the tiny planet, a half-bend, and the unroll.
        let polar = |conversion: u32, interp: f32| {
            let mut p = PolarCoordinates::read(Params::EMPTY);
            p.conversion = conversion;
            p.interpolation = interp;
            p.packed()
        };
        let mut planet = lin.clone();
        cpu::polar_coordinates(&mut planet, w, h, &polar(0, 100.0));
        write("5-polar-tiny-planet", &to_srgb(&planet));
        let mut halfway = lin.clone();
        cpu::polar_coordinates(&mut halfway, w, h, &polar(0, 50.0));
        write("6-polar-half-bent", &to_srgb(&halfway));
        let mut unrolled = lin.clone();
        cpu::polar_coordinates(&mut unrolled, w, h, &polar(1, 100.0));
        write("7-polar-unrolled", &to_srgb(&unrolled));

        // ---- Twirl: a gentle spiral on the frame centre, and a hard one
        // off-centre — the rim must not show a ring in either.
        let twirl = |angle: f32, radius: f32, cx: f32, cy: f32| {
            let mut t = Twirl::read(Params::EMPTY);
            t.angle = angle;
            // Radius is px@comp; the resolve step would have scaled it to the
            // raster, so the proof writes raster pixels, as a fraction of the
            // diagonal for a size-independent picture.
            t.radius = radius * (fw * fw + fh * fh).sqrt();
            t.centre_x = cx * fw;
            t.centre_y = cy * fh;
            t.packed()
        };
        let mut gentle = lin.clone();
        cpu::twirl(&mut gentle, w, h, &twirl(90.0, 0.30, 0.5, 0.5));
        write("8-twirl-gentle", &to_srgb(&gentle));
        let mut hard = lin.clone();
        cpu::twirl(&mut hard, w, h, &twirl(-320.0, 0.22, 0.33, 0.6));
        write("9-twirl-hard-offcentre", &to_srgb(&hard));

        // ---- Spherize: the glass ball, and its exact opposite.
        let ball = |bulge: f32, radius: f32| {
            let mut s = Spherize::read(Params::EMPTY);
            s.bulge = bulge;
            s.radius = radius * (fw * fw + fh * fh).sqrt();
            s.centre_x = fw * 0.5;
            s.centre_y = fh * 0.5;
            s.packed()
        };
        let mut bulged = lin.clone();
        cpu::spherize(&mut bulged, w, h, &ball(100.0, 0.20));
        write("10-spherize-bulge", &to_srgb(&bulged));
        let mut pinched = lin.clone();
        cpu::spherize(&mut pinched, w, h, &ball(-100.0, 0.20));
        write("11-spherize-pinch", &to_srgb(&pinched));
    }
}

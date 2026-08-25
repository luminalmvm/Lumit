//! Visual proof for Wave 2's Transitions batch (docs/08 §3.70–§3.72): render one
//! real frame through each of the three effects at two settings, so the kernels
//! can be judged by eye rather than asserted.
//!
//! # In plain terms
//!
//! The oracle tests prove the graphics card and the CPU agree. They cannot prove
//! the picture is *right* — an effect that renders black, or the input
//! unchanged, agrees with a reference that does the same wrong thing. Three
//! things here have to be looked at, and each has a distinctive look it either
//! reads as or does not:
//!
//! 1. **Venetian blinds** must be a rank of *hard slats* — many parallel bands
//!    of picture with empty gaps between them, all the same width, and the whole
//!    rank must turn together when Direction does. One soft edge across the
//!    frame is the failure.
//! 2. **Iris wipe** must cut a *polygon-shaped hole* with straight sides and
//!    visible corners, and with Use inner radius on it must be a *star*. A
//!    circle means the sector fold collapsed.
//! 3. **Card wipe** must read as *cards turning*, not as cards squashing: each
//!    one narrows towards one edge while the picture on it slides, and a card
//!    that has not started must be untouched. All of them fading together is
//!    the failure.
//!
//! Ignored by default — it wants real footage and writes files. Run with:
//!
//! ```text
//! LUMIT_TRANSITION_PROOF_CLIPS="C:/tmp/lumit-shots/Gameplay.mp4" \
//! LUMIT_TRANSITION_PROOF_OUT="C:/tmp/lumit-shots" \
//!   cargo test -p lumit-render --release --test transition_proof -- --ignored --nocapture
//! ```
//!
//! `LUMIT_TRANSITION_PROOF_FRAME` picks the frame (default 0). Output is raw
//! RGBA8 (`<name>.<w>x<h>.raw`), for the reason `blur_proof.rs` gives: nothing in
//! the workspace encodes PNG, and a throwaway encoder written for a diagnostic is
//! exactly the code that should not exist. The runner converts them.

use lumit_core::fx::cpu;
use lumit_core::fx::effects::{
    card_wipe::CardWipe, iris_wipe::IrisWipe, venetian_blinds::VenetianBlinds,
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

/// Flatten premultiplied RGBA over a mid-grey card, so what a transition has
/// taken away reads as grey in a viewer rather than as whatever the viewer
/// decides to show. `utility_proof.rs`'s helper, for its reason.
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

#[test]
#[ignore = "harness: set LUMIT_TRANSITION_PROOF_CLIPS; writes raw RGBA files"]
fn render_the_three_transition_effects() {
    let Ok(clips) = std::env::var("LUMIT_TRANSITION_PROOF_CLIPS") else {
        eprintln!("set LUMIT_TRANSITION_PROOF_CLIPS to ;-separated clip paths");
        return;
    };
    let out_dir = std::env::var("LUMIT_TRANSITION_PROOF_OUT")
        .unwrap_or_else(|_| "C:/tmp/lumit-shots".to_owned());
    let frame: usize = std::env::var("LUMIT_TRANSITION_PROOF_FRAME")
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
            let name = format!("{out_dir}/{stem}.t{frame}.{tag}.{w}x{h}.raw");
            match std::fs::write(&name, px) {
                Ok(()) => eprintln!("wrote {name}"),
                Err(e) => eprintln!("could not write {name}: {e}"),
            }
        };
        write("0-source", &a.rgba);

        // ---- Venetian blinds: hard slats, then a turned and feathered rank.
        let vb = |completion: f32, direction: f32, width: f32, feather: f32| {
            let mut v = VenetianBlinds::read(Params::EMPTY);
            v.completion = completion;
            v.direction = direction;
            v.width = width;
            v.feather = feather;
            v
        };
        let mut slats = lin.clone();
        cpu::venetian_blinds(&mut slats, w, h, &vb(55.0, 0.0, fh * 0.06, 0.0).packed());
        write("1-blinds-hard", &to_srgb(&over_grey(&slats)));
        let mut turned = lin.clone();
        cpu::venetian_blinds(
            &mut turned,
            w,
            h,
            &vb(45.0, 30.0, fh * 0.09, fh * 0.012).packed(),
        );
        write("2-blinds-turned-soft", &to_srgb(&over_grey(&turned)));

        // ---- Iris wipe: a hexagon, then a twelve-point star.
        // The two radii are px@comp in the schema; the resolve step scales
        // them, so a test that calls `packed` directly writes raster pixels.
        let iw = |points: i32, outer: f32, inner: Option<f32>, rotation: f32, feather: f32| {
            let mut i = IrisWipe::read(Params::EMPTY);
            i.centre_x = fw * 0.5;
            i.centre_y = fh * 0.5;
            i.points = points;
            i.outer_radius = outer;
            i.use_inner_radius = inner.is_some();
            i.inner_radius = inner.unwrap_or(0.0);
            i.rotation = rotation;
            i.feather = feather;
            i
        };
        let mut hexagon = lin.clone();
        cpu::iris_wipe(
            &mut hexagon,
            w,
            h,
            &iw(6, fh * 0.34, None, 0.0, 0.0).packed(),
        );
        write("3-iris-hexagon", &to_srgb(&over_grey(&hexagon)));
        let mut star = lin.clone();
        cpu::iris_wipe(
            &mut star,
            w,
            h,
            &iw(12, fh * 0.46, Some(fh * 0.19), 15.0, fh * 0.008).packed(),
        );
        write("4-iris-star", &to_srgb(&over_grey(&star)));

        // ---- Card wipe: an ordered wave, then a shuffled one with mixed axes.
        let cw = |completion: f32,
                  rows: i32,
                  columns: i32,
                  width: f32,
                  order: u32,
                  axis: u32,
                  direction: u32,
                  randomness: f32| {
            let mut c = CardWipe::read(Params::EMPTY);
            c.completion = completion;
            c.rows = rows;
            c.columns = columns;
            c.transition_width = width;
            c.flip_order = order;
            c.flip_axis = axis;
            c.flip_direction = direction;
            c.randomness = randomness;
            c.seed = 20_260_820;
            c
        };
        let mut wave = lin.clone();
        cpu::card_wipe(
            &mut wave,
            w,
            h,
            // Transition width is px@comp since K-558: 45 % of the frame's
            // width, which is the axis the Left-to-right order runs along.
            &cw(50.0, 4, 7, fw * 0.45, 0, 0, 0, 0.0).packed(fw, fh),
        );
        write("5-cards-wave", &to_srgb(&over_grey(&wave)));
        let mut shuffled = lin.clone();
        cpu::card_wipe(
            &mut shuffled,
            w,
            h,
            // Top-to-bottom, so a quarter of the frame's *height*.
            &cw(55.0, 6, 10, fh * 0.25, 2, 2, 2, 70.0).packed(fw, fh),
        );
        write("6-cards-shuffled", &to_srgb(&over_grey(&shuffled)));
    }
}

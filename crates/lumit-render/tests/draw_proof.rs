//! Visual proof for Wave 2's Draw and grain batch (docs/08 §3.73–§3.77): render
//! one real frame through each of the five effects at two settings, so the
//! kernels can be judged by eye rather than asserted.
//!
//! # In plain terms
//!
//! The oracle tests prove the graphics card and the CPU agree. They cannot prove
//! the picture is *right* — an effect that renders black, or the input
//! unchanged, agrees with a reference that does the same wrong thing. Five
//! things here have to be looked at, and each has a distinctive look it either
//! reads as or does not:
//!
//! 1. **Beam** must be a *tapered shaft* running between the two points, fat at
//!    one end and thin at the other, with a visible rim colour around a brighter
//!    core. A stripe of even width, or one colour only, is the failure.
//! 2. **Lightning** must be a *jagged forked bolt*, not a smooth curve and not a
//!    straight line: creases along its length, branches leaving it, and a halo
//!    around a bright filament.
//! 3. **Radio waves** must be *several* outlines at different sizes, concentric
//!    about the producer point, fading with age — and with Star on, star-shaped.
//!    One ring, or a filled disc, is the failure.
//! 4. **Vegas** must be *dashes lying along the picture's contours*, following
//!    the shapes in the frame rather than running straight across it.
//! 5. **Add grain** must be *visible texture that follows the tone*: mottling in
//!    the mid tones, near-clean blacks and whites at the default weights, and a
//!    grain that is plainly bigger than one pixel at Size 4.
//!
//! Ignored by default — it wants real footage and writes files. Run with:
//!
//! ```text
//! LUMIT_DRAW_PROOF_CLIPS="C:/tmp/lumit-shots/Gameplay.mp4" \
//! LUMIT_DRAW_PROOF_OUT="C:/tmp/lumit-shots" \
//!   cargo test -p lumit-render --release --test draw_proof -- --ignored --nocapture
//! ```
//!
//! `LUMIT_DRAW_PROOF_FRAME` picks the frame (default 0). Output is raw RGBA8
//! (`<name>.<w>x<h>.raw`), for the reason `blur_proof.rs` gives: nothing in the
//! workspace encodes PNG, and a throwaway encoder written for a diagnostic is
//! exactly the code that should not exist. The runner converts them.

use lumit_core::fx::cpu;
use lumit_core::fx::effects::{
    add_grain::AddGrain, beam::Beam, lightning::Lightning, radio_waves::RadioWaves, vegas::Vegas,
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

/// Flatten premultiplied RGBA over a mid-grey card, so a draw effect with
/// Composite on original off reads as a shape on grey rather than as whatever
/// the viewer decides to show. `transition_proof.rs`'s helper, for its reason.
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
#[ignore = "harness: set LUMIT_DRAW_PROOF_CLIPS; writes raw RGBA files"]
fn render_the_five_draw_effects() {
    let Ok(clips) = std::env::var("LUMIT_DRAW_PROOF_CLIPS") else {
        eprintln!("set LUMIT_DRAW_PROOF_CLIPS to ;-separated clip paths");
        return;
    };
    let out_dir =
        std::env::var("LUMIT_DRAW_PROOF_OUT").unwrap_or_else(|_| "C:/tmp/lumit-shots".to_owned());
    let frame: usize = std::env::var("LUMIT_DRAW_PROOF_FRAME")
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

        // ---- Beam: the whole shaft, then a short one part way along its run.
        // Length is px@comp, so "the whole shaft" is the run's own length
        // rather than a hundred per cent of it.
        let run = (fw * 0.76).hypot(fh * 0.64);
        let bm = |length: f32, time: f32, t0: f32, t1: f32, soft: f32| {
            let mut b = Beam::read(Params::EMPTY);
            b.start_x = fw * 0.12;
            b.start_y = fh * 0.82;
            b.end_x = fw * 0.88;
            b.end_y = fh * 0.18;
            b.length = length;
            b.time = time;
            b.start_thickness = t0;
            b.end_thickness = t1;
            b.softness = soft;
            b
        };
        let mut full = lin.clone();
        cpu::beam(
            &mut full,
            w,
            h,
            &bm(run, 100.0, fh * 0.045, fh * 0.008, 45.0).packed(),
        );
        write("1-beam-full", &to_srgb(&over_grey(&full)));
        let mut shot = lin.clone();
        cpu::beam(
            &mut shot,
            w,
            h,
            &bm(run * 0.25, 70.0, fh * 0.014, fh * 0.055, 70.0).packed(),
        );
        write("2-beam-shot", &to_srgb(&over_grey(&shot)));

        // ---- Lightning: a strike between two points, then an omni burst.
        let lt = |kind: u32, amp: f32, forking: f32, core: f32, glow: f32| {
            let mut l = Lightning::read(Params::EMPTY);
            l.origin_x = fw * 0.15;
            l.origin_y = fh * 0.85;
            l.direction_x = fw * 0.85;
            l.direction_y = fh * 0.15;
            l.lightning_type = kind;
            l.amplitude = amp;
            l.forking = forking;
            l.core_radius = core;
            l.glow_radius = glow;
            l.seed = 20_260_820;
            l
        };
        let mut strike = lin.clone();
        cpu::lightning(
            &mut strike,
            w,
            h,
            &lt(1, 14.0, 60.0, fh * 0.003, fh * 0.022).packed(),
        );
        write("3-lightning-strike", &to_srgb(&over_grey(&strike)));
        let mut omni = lin.clone();
        let mut o = lt(2, 20.0, 90.0, fh * 0.002, fh * 0.016);
        o.origin_x = fw * 0.5;
        o.origin_y = fh * 0.5;
        o.direction_x = fw * 0.5;
        o.direction_y = fh * 0.1;
        o.conductivity = 37.0;
        cpu::lightning(&mut omni, w, h, &o.packed());
        write("4-lightning-omni", &to_srgb(&over_grey(&omni)));

        // ---- Radio waves: circles pinging out, then a spinning star.
        let rw = |sides: i32, star: bool, spin: f32, time: f32| {
            let mut r = RadioWaves::read(Params::EMPTY);
            r.centre_x = fw * 0.5;
            r.centre_y = fh * 0.55;
            r.sides = sides;
            r.star = star;
            r.spin = spin;
            r.time = time;
            r.expansion = fh * 0.22;
            r.stroke_width = fh * 0.005;
            r
        };
        let mut ping = lin.clone();
        cpu::radio_waves(&mut ping, w, h, &rw(48, false, 0.0, 3.4).packed());
        write("5-waves-ping", &to_srgb(&over_grey(&ping)));
        let mut star = lin.clone();
        let mut s = rw(7, true, 40.0, 3.9);
        s.star_depth = 45.0;
        s.frequency = 1.4;
        s.lifespan = 3.0;
        cpu::radio_waves(&mut star, w, h, &s.packed());
        write("6-waves-star", &to_srgb(&over_grey(&star)));

        // ---- Vegas: a continuous outline, then marching dashes.
        let vg = |length: f32, seg: f32, width: f32, threshold: f32| {
            let mut v = Vegas::read(Params::EMPTY);
            v.length = length;
            v.segment_length = seg;
            v.width = width;
            v.threshold = threshold;
            v
        };
        let mut outline = lin.clone();
        cpu::vegas(
            &mut outline,
            w,
            h,
            &vg(100.0, 100.0, fh * 0.004, 45.0).packed(),
        );
        write("7-vegas-outline", &to_srgb(&over_grey(&outline)));
        let mut dashes = lin.clone();
        cpu::vegas(
            &mut dashes,
            w,
            h,
            &vg(45.0, fh * 0.06, fh * 0.006, 55.0).packed(),
        );
        write("8-vegas-dashes", &to_srgb(&over_grey(&dashes)));

        // ---- Add grain: a fine stock, then a coarse mid-tone-only one.
        let ag = |intensity: f32, size: f32, soft: f32| {
            let mut g = AddGrain::read(Params::EMPTY);
            g.intensity = intensity;
            g.size = size;
            g.softness = soft;
            g.seed = 20_260_820;
            g
        };
        let mut fine = lin.clone();
        cpu::add_grain(&mut fine, w, h, &ag(90.0, 1.5, 40.0).packed(0));
        write("9-grain-fine", &to_srgb(&fine));
        let mut coarse = lin.clone();
        let mut c = ag(200.0, 5.0, 80.0);
        c.shadows = 0.0;
        c.highlights = 0.0;
        c.monochrome = true;
        cpu::add_grain(&mut coarse, w, h, &c.packed(0));
        write("10-grain-coarse", &to_srgb(&coarse));
    }
}

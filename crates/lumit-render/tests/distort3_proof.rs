//! Visual proof for Wave 2's Distort II batch (docs/08 §3.53–§3.57): render one
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
//! 1. **Ripple** must show concentric rings, strongest in a band a third of the
//!    way out and dying at the rim, with **no pinch at the epicentre**.
//! 2. **Wave warp** must wave: the sine one like a flag, the square one as hard
//!    slices, and a pinned edge must be visibly nailed down.
//! 3. **Bezier warp** must bow its edges, and the frame outside the patch must be
//!    empty rather than smeared.
//! 4. **Warp**'s styles must each read as their own name — an Arc must arc, a
//!    Fisheye must bulge round, a Twist must turn the top against the bottom.
//! 5. **Roughen edges** must chew the outline of a shape and leave its middle
//!    exactly alone.
//!
//! Ignored by default — it wants real footage and writes files. Run with:
//!
//! ```text
//! LUMIT_DISTORT3_PROOF_CLIPS="C:/tmp/lumit-shots/Gameplay.mp4" \
//! LUMIT_DISTORT3_PROOF_OUT="C:/tmp/lumit-shots" \
//!   cargo test -p lumit-render --release --test distort3_proof -- --ignored --nocapture
//! ```
//!
//! `LUMIT_DISTORT3_PROOF_FRAME` picks the frame (default 0). Output is raw RGBA8
//! (`<name>.<w>x<h>.raw`), for the reason `blur_proof.rs` gives: nothing in the
//! workspace encodes PNG, and a throwaway encoder written for a diagnostic is
//! exactly the code that should not exist. The runner converts them.

use lumit_core::fx::cpu;
use lumit_core::fx::effects::{
    bezier_warp::BezierWarp, ripple::Ripple, roughen_edges::RoughenEdges, warp::Warp,
    wave_warp::WaveWarp,
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

/// A rounded rectangle cut out of the plate, premultiplied: the shape Roughen
/// edges needs, since a full-frame opaque plate has no outline of its own to
/// chew but the frame's own border.
fn rounded_card(lin: &[f32], w: u32, h: u32) -> Vec<f32> {
    let mut out = lin.to_vec();
    let (fw, fh) = (w as f32, h as f32);
    let (hx, hy) = (fw * 0.30, fh * 0.30);
    let r = fh * 0.12;
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let dx = ((x as f32 + 0.5) - fw * 0.5).abs() - (hx - r);
            let dy = ((y as f32 + 0.5) - fh * 0.5).abs() - (hy - r);
            let d = (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt() + dx.max(dy).min(0.0) - r;
            let a = (0.5 - d).clamp(0.0, 1.0);
            for c in 0..4 {
                out[i + c] *= a;
            }
        }
    }
    out
}

#[test]
#[ignore = "harness: set LUMIT_DISTORT3_PROOF_CLIPS; writes raw RGBA files"]
fn render_the_five_distort_three_effects() {
    let Ok(clips) = std::env::var("LUMIT_DISTORT3_PROOF_CLIPS") else {
        eprintln!("set LUMIT_DISTORT3_PROOF_CLIPS to ;-separated clip paths");
        return;
    };
    let out_dir = std::env::var("LUMIT_DISTORT3_PROOF_OUT")
        .unwrap_or_else(|_| "C:/tmp/lumit-shots".to_owned());
    let frame: usize = std::env::var("LUMIT_DISTORT3_PROOF_FRAME")
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
        let diag = (fw * fw + fh * fh).sqrt();

        let write = |tag: &str, px: &[u8]| {
            let name = format!("{out_dir}/{stem}.p3{frame}.{tag}.{w}x{h}.raw");
            match std::fs::write(&name, px) {
                Ok(()) => eprintln!("wrote {name}"),
                Err(e) => eprintln!("could not write {name}: {e}"),
            }
        };
        write("0-source", &a.rgba);

        // ---- Ripple: a wide asymmetric spread, and a tight symmetric one part
        // way through its evolution. Neither may pinch at the epicentre.
        let ripple = |asym: u32, radius: f32, height: f32, width: f32, evo: f32| {
            let mut r = Ripple::read(Params::EMPTY);
            r.wave_type = asym;
            // The three lengths are % diag; the resolve step would have made
            // them pixels.
            r.radius = radius * diag;
            r.wave_height = height * diag;
            r.wave_width = width * diag;
            r.evolution = evo;
            r.centre_x = fw * 0.5;
            r.centre_y = fh * 0.5;
            r.packed()
        };
        let mut wide = lin.clone();
        cpu::ripple(&mut wide, w, h, &ripple(1, 0.36, 0.008, 0.05, 0.0));
        write("1-ripple-asymmetric", &to_srgb(&wide));
        let mut tight = lin.clone();
        cpu::ripple(&mut tight, w, h, &ripple(0, 0.24, 0.008, 0.022, 120.0));
        write("2-ripple-symmetric-tight", &to_srgb(&tight));

        // ---- Wave warp: a sine flag, and a hard square wave with its left and
        // right edges pinned — the pinned columns must be visibly still.
        let wave = |shape: u32, height: f32, width: f32, dir: f32, pin: u32| {
            let mut v = WaveWarp::read(Params::EMPTY);
            v.wave_type = shape;
            v.wave_height = height;
            v.wave_width = width;
            v.direction = dir;
            v.pinning = pin;
            v.packed()
        };
        let mut flag = lin.clone();
        cpu::wave_warp(&mut flag, w, h, &wave(0, 55.0, 340.0, 0.0, 0));
        write("3-wavewarp-sine-flag", &to_srgb(&flag));
        let mut slices = lin.clone();
        cpu::wave_warp(&mut slices, w, h, &wave(1, 70.0, 200.0, 0.0, 2));
        write("4-wavewarp-square-pinned", &to_srgb(&slices));

        // ---- Bezier warp: a banner (both horizontal edges bowed the same way),
        // and a pulled corner with an inward bow, whose outside must be empty.
        let identity = || {
            let mut b = BezierWarp::read(Params::EMPTY);
            b.upper_left_x = 0.0;
            b.upper_left_y = 0.0;
            b.upper_right_x = fw;
            b.upper_right_y = 0.0;
            b.lower_right_x = fw;
            b.lower_right_y = fh;
            b.lower_left_x = 0.0;
            b.lower_left_y = fh;
            b.top_left_tangent_x = fw / 3.0;
            b.top_left_tangent_y = 0.0;
            b.top_right_tangent_x = fw * 2.0 / 3.0;
            b.top_right_tangent_y = 0.0;
            b.right_top_tangent_x = fw;
            b.right_top_tangent_y = fh / 3.0;
            b.right_bottom_tangent_x = fw;
            b.right_bottom_tangent_y = fh * 2.0 / 3.0;
            b.bottom_left_tangent_x = fw / 3.0;
            b.bottom_left_tangent_y = fh;
            b.bottom_right_tangent_x = fw * 2.0 / 3.0;
            b.bottom_right_tangent_y = fh;
            b.left_top_tangent_x = 0.0;
            b.left_top_tangent_y = fh / 3.0;
            b.left_bottom_tangent_x = 0.0;
            b.left_bottom_tangent_y = fh * 2.0 / 3.0;
            b
        };
        let mut banner = identity();
        banner.top_left_tangent_y = fh * 0.22;
        banner.top_right_tangent_y = fh * 0.22;
        banner.bottom_left_tangent_y = fh * 1.22;
        banner.bottom_right_tangent_y = fh * 1.22;
        let mut bowed = lin.clone();
        cpu::bezier_warp(&mut bowed, w, h, &banner.packed());
        write("5-bezier-banner", &to_srgb(&bowed));
        let mut curl = identity();
        curl.upper_right_x = fw * 0.78;
        curl.upper_right_y = fh * 0.14;
        curl.top_left_tangent_y = fh * -0.10;
        curl.top_right_tangent_x = fw * 0.60;
        curl.right_top_tangent_x = fw * 0.86;
        curl.right_top_tangent_y = fh * 0.30;
        let mut pulled = lin.clone();
        cpu::bezier_warp(&mut pulled, w, h, &curl.packed());
        write("6-bezier-pulled-corner", &to_srgb(&pulled));

        // ---- Warp: four of the thirteen, chosen because each has to read as its
        // own name — an arc, a flag, a round fisheye and a twist.
        let bend = |style: u32, amount: f32, hd: f32, vd: f32| {
            let mut a = Warp::read(Params::EMPTY);
            a.style = style;
            a.bend = amount;
            a.horizontal_distortion = hd;
            a.vertical_distortion = vd;
            a.packed()
        };
        for (tag, style, amount, hd, vd) in [
            ("7-warp-arc", 0u32, 70.0, 0.0, 0.0),
            ("8-warp-flag", 5, 80.0, 0.0, 0.0),
            ("9-warp-fisheye", 9, 90.0, 0.0, 0.0),
            ("10-warp-twist", 12, 55.0, 0.0, 0.0),
        ] {
            let mut bent = lin.clone();
            cpu::warp(&mut bent, w, h, &bend(style, amount, hd, vd));
            write(tag, &to_srgb(&bent));
        }

        // ---- Roughen edges: on a rounded card cut out of the plate, so there is
        // a real outline to chew and a real middle that must survive untouched.
        let card = rounded_card(&lin, w, h);
        write("11-card-source", &to_srgb(&card));
        let rough = |edge: u32, border: f32, scale: f32, infl: f32, colour: bool| {
            let mut r = RoughenEdges::read(Params::EMPTY);
            r.edge_type = edge;
            r.border = border;
            r.scale = scale;
            r.fractal_influence = infl;
            r.complexity = 3;
            r.offset_x = fw * 0.5;
            r.offset_y = fh * 0.5;
            r.colour_edge = colour;
            r.edge_colour = [0.9, 0.35, 0.05, 1.0];
            r.seed = 5;
            r.packed()
        };
        let mut torn = card.clone();
        cpu::roughen_edges(&mut torn, w, h, &rough(0, 60.0, 130.0, 100.0, false));
        write("12-roughen-torn", &to_srgb(&torn));
        let mut burnt = card.clone();
        cpu::roughen_edges(&mut burnt, w, h, &rough(2, 90.0, 70.0, 160.0, true));
        write("13-roughen-spiky-coloured", &to_srgb(&burnt));
    }
}

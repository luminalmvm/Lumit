//! Visual proof for Wave 2's Stylise II batch (docs/08 §3.64–§3.69): render one
//! real frame through each of the six effects at two settings, so the kernels
//! can be judged by eye rather than asserted.
//!
//! # In plain terms
//!
//! The oracle tests prove the graphics card and the CPU agree. They cannot prove
//! the picture is *right* — an effect that renders black, or flat, or the input
//! unchanged, agrees with a reference that does the same wrong thing. Six things
//! here have to be looked at:
//!
//! 1. **Median** must flatten the picture into paint-like patches while leaving
//!    the edges where they were — not soften them, which is what would say a
//!    blur had been written by mistake.
//! 2. **Mosaic** must be flat rectangular blocks on a regular grid, and the
//!    sharp mode must be visibly crisper than the averaged one.
//! 3. **Find edges** must read as a pencil drawing on white, and its Invert as
//!    glowing lines on black.
//! 4. **Emboss** must be AE's grey relief — a flat mid-grey sheet with the
//!    frame's edges raised out of it, lit from Direction.
//! 5. **Texturize** must look like the picture printed on the chosen texture,
//!    with the texture's weave lit from one side.
//! 6. **Broadcast safe** must visibly pull the hottest, most saturated parts of
//!    the frame down, and its key modes must cut the frame into the legal half
//!    and the illegal one.
//!
//! Ignored by default — it wants real footage and writes files. Run with:
//!
//! ```text
//! LUMIT_STYLISE2_PROOF_CLIPS="C:/tmp/lumit-shots/Gameplay.mp4" \
//! LUMIT_STYLISE2_PROOF_OUT="C:/tmp/lumit-shots" \
//!   cargo test -p lumit-render --release --test stylise2_proof -- --ignored --nocapture
//! ```
//!
//! `LUMIT_STYLISE2_PROOF_FRAME` picks the frame (default 0). Output is raw RGBA8
//! (`<name>.<w>x<h>.raw`), for the reason `blur_proof.rs` gives: nothing in the
//! workspace encodes PNG, and a throwaway encoder written for a diagnostic is
//! exactly the code that should not exist. The runner converts them.

use lumit_core::fx::cpu;
use lumit_core::fx::effects::{
    broadcast_safe::BroadcastSafe, emboss::Emboss, find_edges::FindEdges, median::Median,
    mosaic::Mosaic, texturize::Texturize,
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

/// A canvas: a coarse woven weave with a little grain in it, so Texturize has
/// something with real relief to press in. Written here rather than loaded,
/// because a proof that needs a file nobody has is a proof nobody runs.
fn canvas(w: u32, h: u32) -> Vec<f32> {
    let mut t = vec![0.0f32; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let u = x as f32;
            let v = y as f32;
            // A weave of about a 45-pixel pitch: coarse enough that a relief of
            // a few pixels has a real gradient to read.
            let weave = (u * 0.14).sin() * 0.5 + (v * 0.14).cos() * 0.5;
            // A little hash grain, so the weave is not a pure sine.
            let n = ((x.wrapping_mul(374_761_393) ^ y.wrapping_mul(668_265_263)) >> 8) & 0xff;
            let grain = f32::from(n as u8) / 255.0 - 0.5;
            let g = (0.5 + 0.22 * weave + 0.08 * grain).clamp(0.0, 1.0);
            t[i] = g;
            t[i + 1] = g;
            t[i + 2] = g;
            t[i + 3] = 1.0;
        }
    }
    t
}

/// The mean Rec. 709 luma of a linear frame, printed beside each render so the
/// Broadcast safe claim is not judged purely by eye.
fn mean_luma(lin: &[f32]) -> f32 {
    let n = (lin.len() / 4) as f32;
    lin.chunks_exact(4)
        .map(|p| p[0] * 0.2126 + p[1] * 0.7152 + p[2] * 0.0722)
        .sum::<f32>()
        / n
}

/// How much of the frame is over a broadcast limit, as a fraction — the other
/// half of Broadcast safe's proof.
///
/// Measured on **unpremultiplied** colour, as the effect measures it: a
/// half-transparent white is a white pixel that happens to be half covered, and
/// asking whether its premultiplied value is legal answers a different question.
/// The tolerance is a ten-thousandth, because a pixel the repair landed exactly
/// on the limit lands a bit either side of it.
fn illegal_fraction(lin: &[f32], target: f32) -> f32 {
    let n = (lin.len() / 4) as f32;
    let hot = lin
        .chunks_exact(4)
        .filter(|p| {
            if p[3] <= 0.0 {
                return false;
            }
            let v = [
                (p[0] / p[3]).max(0.0).sqrt(),
                (p[1] / p[3]).max(0.0).sqrt(),
                (p[2] / p[3]).max(0.0).sqrt(),
            ];
            let y = v[0] * 0.2126 + v[1] * 0.7152 + v[2] * 0.0722;
            y + cpu::broadcast_chroma(v, y) > target + 1e-4
        })
        .count() as f32;
    hot / n
}

#[test]
#[ignore = "harness: set LUMIT_STYLISE2_PROOF_CLIPS; writes raw RGBA files"]
fn render_the_six_stylise_two_effects() {
    let Ok(clips) = std::env::var("LUMIT_STYLISE2_PROOF_CLIPS") else {
        eprintln!("set LUMIT_STYLISE2_PROOF_CLIPS to ;-separated clip paths");
        return;
    };
    let out_dir = std::env::var("LUMIT_STYLISE2_PROOF_OUT")
        .unwrap_or_else(|_| "C:/tmp/lumit-shots".to_owned());
    let frame: usize = std::env::var("LUMIT_STYLISE2_PROOF_FRAME")
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

        let write = |tag: &str, px: &[u8]| {
            let name = format!("{out_dir}/{stem}.s2{frame}.{tag}.{w}x{h}.raw");
            match std::fs::write(&name, px) {
                Ok(()) => eprintln!("wrote {name}"),
                Err(e) => eprintln!("could not write {name}: {e}"),
            }
        };
        write("0-source", &a.rgba);
        eprintln!("source mean luma {:.4}", mean_luma(&lin));

        // ---- Median: a gentle despeckle and the paint-like flattening at the
        // cap. The edges must stay where they are.
        for (tag, radius) in [("1-median-1", 1.0f32), ("2-median-3", 3.0)] {
            let mut m = Median::read(Params::EMPTY);
            m.radius = radius;
            let mut out = lin.clone();
            cpu::median(&mut out, w, h, &m.packed());
            write(tag, &to_srgb(&out));
        }

        // ---- Mosaic: the averaged default and a coarse sharp-coloured grid.
        for (tag, bx, by, sharp) in [
            ("3-mosaic-averaged", 48, 27, false),
            ("4-mosaic-sharp", 16, 9, true),
        ] {
            let mut m = Mosaic::read(Params::EMPTY);
            m.horizontal_blocks = bx;
            m.vertical_blocks = by;
            m.sharp_colours = sharp;
            let mut out = lin.clone();
            cpu::mosaic(&mut out, w, h, &m.packed());
            write(tag, &to_srgb(&out));
        }

        // ---- Find edges: AE's pencil drawing, and the inverted glow.
        for (tag, invert) in [("5-findedges-drawing", false), ("6-findedges-glow", true)] {
            let mut f = FindEdges::read(Params::EMPTY);
            f.invert = invert;
            let (iv, mix) = f.packed();
            let mut out = lin.clone();
            cpu::find_edges(&mut out, w, h, iv, mix);
            write(tag, &to_srgb(&out));
        }

        // ---- Emboss: the shipped default, and a deep relief lit from the other
        // side. Both must be grey, and the two must be lit oppositely.
        for (tag, direction, relief, contrast) in [
            ("7-emboss-default", 45.0f32, 2.0f32, 100.0f32),
            ("8-emboss-deep", 225.0, 5.0, 180.0),
        ] {
            let mut e = Emboss::read(Params::EMPTY);
            e.direction = direction;
            e.relief = relief;
            e.contrast = contrast;
            let mut out = lin.clone();
            cpu::emboss(&mut out, w, h, &e.packed());
            write(tag, &to_srgb(&out));
        }

        // ---- Texturize: the canvas stretched over the frame, and the same
        // canvas tiled small and pressed harder.
        let tex = canvas(w, h);
        write("9-texture", &to_srgb(&tex));
        for (tag, placement, scale, relief, contrast) in [
            ("10-texturize-canvas", 0u32, 100.0f32, 4.0f32, 150.0f32),
            ("11-texturize-tiled", 1, 50.0, 6.0, 200.0),
        ] {
            let mut t = Texturize::read(Params::EMPTY);
            t.placement = placement;
            t.scale = scale;
            t.relief = relief;
            t.texture_contrast = contrast;
            let mut out = lin.clone();
            cpu::texturize(&mut out, &tex, w, h, &t.packed());
            write(tag, &to_srgb(&out));
        }

        // ---- Broadcast safe: a hard clamp, and the diagnostic view of what it
        // was clamping. 90 IRE is the strictest the control goes, which is what
        // makes the difference visible on ordinary footage.
        for (tag, mode, max) in [
            ("12-broadcastsafe-brightness", 0u32, 90.0f32),
            ("13-broadcastsafe-saturation", 1, 90.0),
            ("14-broadcastsafe-keyunsafe", 2, 90.0),
        ] {
            let mut b = BroadcastSafe::read(Params::EMPTY);
            b.how_to_treat = mode;
            b.maximum_signal = max;
            let p = b.packed();
            let mut out = lin.clone();
            cpu::broadcast_safe(&mut out, &p);
            eprintln!(
                "{tag}: mean luma {:.4}, illegal before {:.4} after {:.4}",
                mean_luma(&out),
                illegal_fraction(&lin, p.target),
                illegal_fraction(&out, p.target),
            );
            write(tag, &to_srgb(&out));
        }
    }
}

//! Visual proof for the distort batch (docs/08 §3.38–§3.42): render one real
//! frame through each of the five effects at two settings, so the kernels can be
//! judged by eye rather than asserted.
//!
//! # In plain terms
//!
//! The oracle tests prove the graphics card and the CPU agree. They cannot prove
//! the picture is *right* — an effect that renders black, or noise, or the input
//! unchanged, agrees with a reference that does the same wrong thing. Five
//! things here have to be looked at:
//!
//! 1. **Turbulent displace** must swirl, and its pinned edges must stay put
//!    while the middle churns. The matted pair is the one that matters most:
//!    under a ramp matte the warp must *grow* across the frame rather than fade
//!    in, which is the whole claim of the matte override.
//! 2. **Tile** must repeat, and Mirror edges must hide the joins.
//! 3. **Offset** must wrap with no seam and no lost content.
//! 4. **Mirror** must be symmetric about the line, not merely flipped.
//! 5. **Lens distort** must bow straight lines outward, and Reverse must bow
//!    them back — the round trip is the readable proof that the pair inverts.
//!
//! Ignored by default — it wants real footage and writes files. Run with:
//!
//! ```text
//! LUMIT_DISTORT_PROOF_CLIPS="C:/tmp/lumit-shots/Gameplay.mp4" \
//! LUMIT_DISTORT_PROOF_OUT="C:/tmp/lumit-shots" \
//!   cargo test -p lumit-render --release --test distort_proof -- --ignored --nocapture
//! ```
//!
//! `LUMIT_DISTORT_PROOF_FRAME` picks the frame (default 0). Output is raw RGBA8
//! (`<name>.<w>x<h>.raw`), for the reason `blur_proof.rs` gives: nothing in the
//! workspace encodes PNG, and a throwaway encoder written for a diagnostic is
//! exactly the code that should not exist. The runner converts them.

use lumit_core::fx::cpu;
use lumit_core::fx::effects::{
    lens_distort::LensDistort, mirror::Mirror, offset::Offset, tile::Tile,
    turbulent_displace::TurbulentDisplace,
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

/// A left-to-right ramp of matte, opaque: every strength at once, so one picture
/// answers "does the matte scale the push or fade the result?".
fn ramp_matte(w: u32, h: u32) -> Vec<f32> {
    let mut m = vec![0.0f32; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let k = x as f32 / (w - 1).max(1) as f32;
            m[i] = k;
            m[i + 1] = k;
            m[i + 2] = k;
            m[i + 3] = 1.0;
        }
    }
    m
}

/// The generic strength dissolve, for comparison: the effect in full, lerped
/// back towards its input by the matte's luma (docs/08 §2.6).
fn dissolved(input: &[f32], processed: &mut [f32], matte: &[f32]) {
    cpu::matte_mix(processed, input, matte, false);
}

#[test]
#[ignore = "harness: set LUMIT_DISTORT_PROOF_CLIPS; writes raw RGBA files"]
fn render_the_five_distort_effects() {
    let Ok(clips) = std::env::var("LUMIT_DISTORT_PROOF_CLIPS") else {
        eprintln!("set LUMIT_DISTORT_PROOF_CLIPS to ;-separated clip paths");
        return;
    };
    let out_dir = std::env::var("LUMIT_DISTORT_PROOF_OUT")
        .unwrap_or_else(|_| "C:/tmp/lumit-shots".to_owned());
    let frame: usize = std::env::var("LUMIT_DISTORT_PROOF_FRAME")
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
            let name = format!("{out_dir}/{stem}.d{frame}.{tag}.{w}x{h}.raw");
            match std::fs::write(&name, px) {
                Ok(()) => eprintln!("wrote {name}"),
                Err(e) => eprintln!("could not write {name}: {e}"),
            }
        };
        write("0-source", &a.rgba);

        // ---- Turbulent displace: a gentle warp and a violent one, then the
        // matted pair that is the point of the override.
        let td = |amount: f32, size: f32, complexity: i32, pinning: u32| {
            let mut t = TurbulentDisplace::read(Params::EMPTY);
            t.amount = amount;
            t.size = size;
            t.complexity = complexity;
            t.pinning = pinning;
            t.offset_x = fw * 0.5;
            t.offset_y = fh * 0.5;
            t.seed = 7;
            t
        };
        let mut gentle = lin.clone();
        cpu::turbulent_displace(
            &mut gentle,
            w,
            h,
            &td(fw * 0.02, fw * 0.25, 3, 1).packed(),
            &[],
        );
        write("1-turbdisplace-gentle", &to_srgb(&gentle));
        let mut violent = lin.clone();
        cpu::turbulent_displace(
            &mut violent,
            w,
            h,
            &td(fw * 0.08, fw * 0.08, 6, 1).packed(),
            &[],
        );
        write("2-turbdisplace-violent", &to_srgb(&violent));

        let matte = ramp_matte(w, h);
        let strong = td(fw * 0.06, fw * 0.15, 4, 0);
        let mut scaled = lin.clone();
        cpu::turbulent_displace(&mut scaled, w, h, &strong.packed(), &matte);
        write("3-turbdisplace-matte-scales", &to_srgb(&scaled));
        let mut faded = lin.clone();
        cpu::turbulent_displace(&mut faded, w, h, &strong.packed(), &[]);
        dissolved(&lin, &mut faded, &matte);
        write("4-turbdisplace-matte-dissolved", &to_srgb(&faded));

        // ---- Tile: the 2x2 default, and a mirrored, phase-shifted 3x3.
        // The four sizes are px@comp, so the shares this proof draws are
        // taken against the frame here rather than typed as per cents.
        let tile = |width: f32, height: f32, mirror: bool, phase: f32, out_w: f32| {
            let mut t = Tile::read(Params::EMPTY);
            t.tile_centre_x = fw * 0.5;
            t.tile_centre_y = fh * 0.5;
            t.tile_width = fw * width;
            t.tile_height = fh * height;
            t.output_width = fw * out_w;
            t.output_height = fh * out_w;
            t.mirror_edges = mirror;
            t.phase = phase;
            t
        };
        let mut plain = lin.clone();
        cpu::tile(
            &mut plain,
            w,
            h,
            &tile(0.5, 0.5, false, 0.0, 1.0).packed(fw, fh),
        );
        write("5-tile-2x2", &to_srgb(&plain));
        let mut fancy = lin.clone();
        cpu::tile(
            &mut fancy,
            w,
            h,
            &tile(0.33, 0.33, true, 180.0, 0.8).packed(fw, fh),
        );
        write("6-tile-mirrored-phased", &to_srgb(&fancy));

        // ---- Offset: a diagonal wrap, and a half-frame wrap (the seam test).
        let offset = |x: f32, y: f32| {
            let mut o = Offset::read(Params::EMPTY);
            o.shift_x = x;
            o.shift_y = y;
            o.packed()
        };
        let mut slid = lin.clone();
        let (shift, mix) = offset(fw * 0.2, fh * 0.15);
        cpu::offset(&mut slid, w, h, shift, mix);
        write("7-offset-diagonal", &to_srgb(&slid));
        let mut halved = lin.clone();
        let (shift, mix) = offset(fw * 0.5, 0.0);
        cpu::offset(&mut halved, w, h, shift, mix);
        write("8-offset-half", &to_srgb(&halved));

        // ---- Mirror: the vertical axis, and a diagonal off-centre one.
        let mirror = |cx: f32, cy: f32, angle: f32| {
            let mut m = Mirror::read(Params::EMPTY);
            m.centre_x = cx;
            m.centre_y = cy;
            m.angle = angle;
            m.packed()
        };
        let mut sym = lin.clone();
        let (c, n, mx) = mirror(fw * 0.5, fh * 0.5, 0.0);
        cpu::mirror(&mut sym, w, h, c, n, mx);
        write("9-mirror-vertical", &to_srgb(&sym));
        let mut diag = lin.clone();
        let (c, n, mx) = mirror(fw * 0.4, fh * 0.6, 45.0);
        cpu::mirror(&mut diag, w, h, c, n, mx);
        write("10-mirror-diagonal", &to_srgb(&diag));

        // ---- Lens distort: a wide barrel, its exact undo, and the round trip.
        let lens = |fov: f32, reverse: bool| {
            let mut l = LensDistort::read(Params::EMPTY);
            l.fov = fov;
            l.reverse = reverse;
            l.centre_x = fw * 0.5;
            l.centre_y = fh * 0.5;
            l.edge = 1; // Repeat, so the corners show the mapping rather than a hole
            l.packed()
        };
        let mut barrel = lin.clone();
        cpu::lens_distort(&mut barrel, w, h, &lens(110.0, false));
        write("11-lens-barrel", &to_srgb(&barrel));
        let mut pincushion = lin.clone();
        cpu::lens_distort(&mut pincushion, w, h, &lens(110.0, true));
        write("12-lens-pincushion", &to_srgb(&pincushion));
        let mut round_trip = barrel.clone();
        cpu::lens_distort(&mut round_trip, w, h, &lens(110.0, true));
        write("13-lens-round-trip", &to_srgb(&round_trip));
    }
}

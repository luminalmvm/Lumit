//! Visual proof for the utility and transition batch (docs/08 §3.43–§3.47):
//! render one real frame through each of the five effects at two settings, so
//! the kernels can be judged by eye rather than asserted.
//!
//! # In plain terms
//!
//! The oracle tests prove the graphics card and the CPU agree. They cannot prove
//! the picture is *right* — an effect that renders black, or noise, or the input
//! unchanged, agrees with a reference that does the same wrong thing. Five
//! things here have to be looked at:
//!
//! 1. **Drop shadow** must put a soft dark copy of the shape *behind* it, down
//!    and to the right at the default 135°, and Shadow only must keep the shadow
//!    alone. A frame of footage is opaque everywhere, so the proof runs on a
//!    shape cut out of it — a shadow needs an alpha edge to be a shadow.
//! 2. **Set matte** must take the shape from the matte, not the strength: under
//!    a disc matte the picture must be a disc with hard content inside it, which
//!    a strength dissolve (drawn beside it) cannot produce.
//! 3. **Channel blur** must soften one channel and leave the others sharp —
//!    visible as colour fringing on a hard edge, not as an overall blur.
//! 4. **Linear wipe** must remove one side cleanly, and feather must soften the
//!    join rather than fade the whole frame.
//! 5. **Radial wipe** must sweep a wedge, and the three directions must differ.
//!
//! Ignored by default — it wants real footage and writes files. Run with:
//!
//! ```text
//! LUMIT_UTILITY_PROOF_CLIPS="C:/tmp/lumit-shots/Gameplay.mp4" \
//! LUMIT_UTILITY_PROOF_OUT="C:/tmp/lumit-shots" \
//!   cargo test -p lumit-render --release --test utility_proof -- --ignored --nocapture
//! ```
//!
//! `LUMIT_UTILITY_PROOF_FRAME` picks the frame (default 0). Output is raw RGBA8
//! (`<name>.<w>x<h>.raw`), for the reason `blur_proof.rs` gives: nothing in the
//! workspace encodes PNG, and a throwaway encoder written for a diagnostic is
//! exactly the code that should not exist. The runner converts them.

use lumit_core::fx::cpu;
use lumit_core::fx::effects::{
    channel_blur::ChannelBlur, drop_shadow::DropShadow, linear_wipe::LinearWipe,
    radial_wipe::RadialWipe,
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

/// The picture cut to a rounded rectangle, premultiplied — a shape, because a
/// drop shadow cast by a frame that is opaque everywhere is a shadow nobody can
/// see.
fn cut_to_a_shape(lin: &[f32], w: u32, h: u32) -> Vec<f32> {
    let mut out = lin.to_vec();
    let (fw, fh) = (w as f32, h as f32);
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let dx = (x as f32 - fw * 0.5).abs() - fw * 0.22;
            let dy = (y as f32 - fh * 0.5).abs() - fh * 0.22;
            let d = dx.max(0.0).hypot(dy.max(0.0)) + dx.max(dy).min(0.0);
            let a = (1.0 - (d - fh * 0.1)).clamp(0.0, 1.0);
            for c in 0..4 {
                out[i + c] *= a;
            }
        }
    }
    out
}

/// A centred disc, opaque — the matte for the Set matte pair.
fn disc_matte(w: u32, h: u32) -> Vec<f32> {
    let mut m = vec![0.0f32; (w * h * 4) as usize];
    let (cx, cy, r) = (w as f32 * 0.5, h as f32 * 0.5, h as f32 * 0.35);
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let d = (x as f32 - cx).hypot(y as f32 - cy);
            let k = (1.0 - (d - r) / (r * 0.15)).clamp(0.0, 1.0);
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

/// Flatten premultiplied RGBA over a mid-grey card, so transparency reads as
/// grey in a viewer rather than as whatever the viewer decides to show.
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
#[ignore = "harness: set LUMIT_UTILITY_PROOF_CLIPS; writes raw RGBA files"]
fn render_the_five_utility_and_transition_effects() {
    let Ok(clips) = std::env::var("LUMIT_UTILITY_PROOF_CLIPS") else {
        eprintln!("set LUMIT_UTILITY_PROOF_CLIPS to ;-separated clip paths");
        return;
    };
    let out_dir = std::env::var("LUMIT_UTILITY_PROOF_OUT")
        .unwrap_or_else(|_| "C:/tmp/lumit-shots".to_owned());
    let frame: usize = std::env::var("LUMIT_UTILITY_PROOF_FRAME")
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
        let fw = w as f32;

        let write = |tag: &str, px: &[u8]| {
            let name = format!("{out_dir}/{stem}.u{frame}.{tag}.{w}x{h}.raw");
            match std::fs::write(&name, px) {
                Ok(()) => eprintln!("wrote {name}"),
                Err(e) => eprintln!("could not write {name}: {e}"),
            }
        };
        write("0-source", &a.rgba);

        // ---- Drop shadow, on a shape cut out of the frame.
        let shape = cut_to_a_shape(&lin, w, h);
        write("1-shape", &to_srgb(&over_grey(&shape)));
        let ds = |distance: f32, softness: f32, only: bool, dir: f32| {
            let mut d = DropShadow::read(Params::EMPTY);
            d.distance = distance;
            d.softness = softness;
            d.shadow_only = only;
            d.direction = dir;
            d
        };
        let mut soft = shape.clone();
        cpu::drop_shadow(
            &mut soft,
            w,
            h,
            &ds(fw * 0.02, fw * 0.012, false, 135.0).packed(),
        );
        write("2-dropshadow-default", &to_srgb(&over_grey(&soft)));
        let mut only = shape.clone();
        cpu::drop_shadow(
            &mut only,
            w,
            h,
            &ds(fw * 0.05, fw * 0.03, true, 315.0).packed(),
        );
        write("3-dropshadow-only", &to_srgb(&over_grey(&only)));

        // ---- Set matte: the matte becomes the shape, which a dissolve cannot do.
        let matte = disc_matte(w, h);
        let mut cut = lin.clone();
        cpu::set_matte(&mut cut, &matte, 0, false, false, 1.0);
        write("4-setmatte-shapes", &to_srgb(&over_grey(&cut)));
        // The same effect under the generic strength dissolve, which is what the
        // Own role buys: dissolving a Set matte gives a frame that is *fainter*
        // in the corners, not a frame cut to the disc.
        let mut faded = lin.clone();
        cpu::set_matte(&mut faded, &matte, 0, false, false, 1.0);
        dissolved(&lin, &mut faded, &matte);
        write("5-setmatte-dissolved", &to_srgb(&over_grey(&faded)));

        // ---- Channel blur: one channel soft, the rest sharp.
        let cb = |r: f32, g: f32, b: f32, alpha: f32| {
            let mut c = ChannelBlur::read(Params::EMPTY);
            c.red = r;
            c.green = g;
            c.blue = b;
            c.alpha = alpha;
            c
        };
        let mut blue = lin.clone();
        let (radii, edge, mix) = cb(0.0, 0.0, fw * 0.012, 0.0).packed();
        cpu::channel_blur(&mut blue, w, h, radii, edge, mix);
        write("6-chanblur-blue", &to_srgb(&blue));
        let mut split = lin.clone();
        let (radii, edge, mix) = cb(fw * 0.02, 0.0, fw * 0.008, 0.0).packed();
        cpu::channel_blur(&mut split, w, h, radii, edge, mix);
        write("7-chanblur-red-and-blue", &to_srgb(&split));

        // ---- Linear wipe: hard and feathered.
        let lw = |completion: f32, angle: f32, feather: f32| {
            let mut l = LinearWipe::read(Params::EMPTY);
            l.centre_x = w as f32 * 0.5;
            l.centre_y = h as f32 * 0.5;
            l.completion = completion;
            l.angle = angle;
            l.feather = feather;
            l
        };
        let mut hard = lin.clone();
        cpu::linear_wipe(&mut hard, w, h, &lw(40.0, 90.0, 0.0).packed());
        write("8-linearwipe-hard", &to_srgb(&over_grey(&hard)));
        let mut soft_wipe = lin.clone();
        cpu::linear_wipe(&mut soft_wipe, w, h, &lw(60.0, 30.0, fw * 0.08).packed());
        write("9-linearwipe-feathered", &to_srgb(&over_grey(&soft_wipe)));

        // ---- Radial wipe: clockwise, and Both feathered.
        let rw = |completion: f32, wipe: u32, feather: f32, start: f32| {
            let mut r = RadialWipe::read(Params::EMPTY);
            r.centre_x = w as f32 * 0.5;
            r.centre_y = h as f32 * 0.5;
            r.completion = completion;
            r.wipe = wipe;
            r.feather = feather;
            r.start_angle = start;
            r
        };
        let mut clock = lin.clone();
        cpu::radial_wipe(&mut clock, w, h, &rw(35.0, 0, 0.0, 0.0).packed());
        write("10-radialwipe-clockwise", &to_srgb(&over_grey(&clock)));
        let mut curtains = lin.clone();
        cpu::radial_wipe(&mut curtains, w, h, &rw(55.0, 2, fw * 0.05, 180.0).packed());
        write("11-radialwipe-both", &to_srgb(&over_grey(&curtains)));
    }
}

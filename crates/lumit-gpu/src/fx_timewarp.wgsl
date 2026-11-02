// Timewarp: the picture between two moments of the input. Mirrors
// lumit_core::fx::effects::timewarp's CPU oracle step for step (the CPU is the
// oracle): a crossfade, or the frame made from the motion between them, after
// lumit-flow's own synthesis.
//
// The motion stays at the size it was measured at (fw by fh) and is read with
// its vectors scaled to the picture, so nothing is brought up to full size
// first. The masks are made at that size too.
//
// Every pass binds the same seven slots; a pass that does not read one is
// handed another texture there, since a binding cannot be left empty.

struct Params {
    phase: f32,   // 0..1, from the first moment towards the second
    mix_amt: f32, // tw_mix only: 0..1 towards b
    w: u32,       // the picture
    h: u32,
    fw: u32,      // the motion
    fh: u32,
    _pad0: u32,
    _pad1: u32,
};

@group(0) @binding(0) var a: texture_2d<f32>;
@group(0) @binding(1) var b: texture_2d<f32>;
// x, y the motion in measured pixels, z how far it is trusted (0..1).
@group(0) @binding(2) var fwd: texture_2d<f32>;
@group(0) @binding(3) var bwd: texture_2d<f32>;
// x how far a pixel of a has no match in b, y the other way round.
@group(0) @binding(4) var occ: texture_2d<f32>;
@group(0) @binding(5) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(6) var<uniform> P: Params;

const OCC_ABS: f32 = 1.5;
const OCC_REL: f32 = 0.05;
const EPS: f32 = 1e-4;

fn load(which: u32, x: i32, y: i32) -> vec4<f32> {
    // Pictures at the picture's size, motion and masks at the motion's.
    var size = vec2<i32>(i32(P.fw), i32(P.fh));
    if (which < 2u) {
        size = vec2<i32>(i32(P.w), i32(P.h));
    }
    let q = clamp(vec2<i32>(x, y), vec2<i32>(0), size - vec2<i32>(1));
    switch which {
        case 0u: { return textureLoad(a, q, 0); }
        case 1u: { return textureLoad(b, q, 0); }
        case 2u: { return textureLoad(fwd, q, 0); }
        case 3u: { return textureLoad(bwd, q, 0); }
        default: { return textureLoad(occ, q, 0); }
    }
}

// Bilinear, edge-clamped, on pixel-centre coordinates (texel i sits at i) of
// that texture's own size.
fn sample(which: u32, p: vec2<f32>) -> vec4<f32> {
    var size = vec2<f32>(f32(P.fw), f32(P.fh));
    if (which < 2u) {
        size = vec2<f32>(f32(P.w), f32(P.h));
    }
    let c = clamp(p, vec2<f32>(0.0), size - vec2<f32>(1.0));
    let f = floor(c);
    let t = c - f;
    let x0 = i32(f.x);
    let y0 = i32(f.y);
    let top = mix(load(which, x0, y0), load(which, x0 + 1, y0), t.x);
    let bot = mix(load(which, x0, y0 + 1), load(which, x0 + 1, y0 + 1), t.x);
    return mix(top, bot, t.y);
}

// How many picture pixels a measured pixel is.
fn scale() -> f32 {
    return f32(P.w) / f32(max(P.fw, 1u));
}

// The motion at picture pixel p, in picture pixels.
fn motion(which: u32, p: vec2<f32>) -> vec2<f32> {
    let s = scale();
    return sample(which, p / s).xy * s;
}

// a towards b by mix_amt.
@compute @workgroup_size(8, 8)
fn tw_mix(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= P.w || gid.y >= P.h) {
        return;
    }
    let q = vec2<i32>(gid.xy);
    let o = textureLoad(a, q, 0);
    textureStore(dst, q, o + (textureLoad(b, q, 0) - o) * P.mix_amt);
}

// How far each field has no partner in the other: the forward-backward
// test, or as much as the field is not trusted. At the motion's size.
fn unmatched(f: vec4<f32>, other: u32, p: vec2<f32>) -> f32 {
    let g = sample(other, p + f.xy).xy;
    let thr = max(OCC_REL * (length(f.xy) + length(g)), OCC_ABS);
    return max(select(0.0, 1.0, length(f.xy + g) > thr), 1.0 - clamp(f.z, 0.0, 1.0));
}

@compute @workgroup_size(8, 8)
fn tw_occ(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= P.fw || gid.y >= P.fh) {
        return;
    }
    let q = vec2<i32>(gid.xy);
    let p = vec2<f32>(gid.xy);
    let oa = unmatched(textureLoad(fwd, q, 0), 3u, p);
    let ob = unmatched(textureLoad(bwd, q, 0), 2u, p);
    textureStore(dst, q, vec4<f32>(oa, ob, 0.0, 0.0));
}

// The masks grown by a pixel, the test missing a boundary's own pixels, then
// averaged over 3 by 3 so the hand-over between the two moments is gradual.
@compute @workgroup_size(8, 8)
fn tw_grow(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= P.fw || gid.y >= P.fh) {
        return;
    }
    var sum = vec2<f32>(0.0);
    for (var cy = -1; cy <= 1; cy++) {
        for (var cx = -1; cx <= 1; cx++) {
            var m = vec2<f32>(0.0);
            for (var dy = -1; dy <= 1; dy++) {
                for (var dx = -1; dx <= 1; dx++) {
                    let x = i32(gid.x) + cx + dx;
                    let y = i32(gid.y) + cy + dy;
                    m = max(m, load(4u, x, y).xy);
                }
            }
            sum += m;
        }
    }
    textureStore(dst, vec2<i32>(gid.xy), vec4<f32>(sum / 9.0, 0.0, 0.0));
}

// Both moments carried along their motion to the phase and blended, each
// trusted where the other cannot see; where neither can, the crossfade.
@compute @workgroup_size(8, 8)
fn tw_synth(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= P.w || gid.y >= P.h) {
        return;
    }
    let q = vec2<i32>(gid.xy);
    let p = vec2<f32>(gid.xy);
    let phi = P.phase;
    let f0 = motion(2u, p);
    let f1 = motion(2u, p - phi * f0);
    let b0 = motion(3u, p);
    let b1 = motion(3u, p - (1.0 - phi) * b0);
    let sa = sample(0u, p - phi * f1);
    let sb = sample(1u, p - (1.0 - phi) * b1);
    let m = sample(4u, p / scale()).xy;
    let plain = mix(textureLoad(a, q, 0), textureLoad(b, q, 0), phi);
    let wa = (1.0 - phi) * (1.0 - m.y) + EPS;
    let wb = phi * (1.0 - m.x) + EPS;
    let warped = (wa * sa + wb * sb) / (wa + wb);
    textureStore(dst, q, mix(warped, plain, m.x * m.y));
}

// The forward motion at the picture's size, in picture pixels, for the
// motion blur.
@compute @workgroup_size(8, 8)
fn tw_field(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= P.w || gid.y >= P.h) {
        return;
    }
    let p = vec2<f32>(gid.xy);
    let s = scale();
    let f = sample(2u, p / s);
    textureStore(dst, vec2<i32>(gid.xy), vec4<f32>(f.xy * s, f.z, 0.0));
}

// Broadcast safe (docs/08-EFFECTS.md §3.69): the signal clamped to a legal
// amplitude. Mirrors lumit_core::fx::cpu::broadcast_safe op-for-op (§1.6: the
// CPU is the oracle).
//
// The pixel is encoded (the batch's sqrt — §3.69 decision 2: the answer here is
// a THRESHOLD, so a last-bit disagreement between the two paths is a pixel keyed
// out on one and not on the other, and sqrt is one correctly-rounded instruction
// on both), its composite amplitude Y + C is measured, and where that is over
// the target one of four things happens.
//
// The standard is not a branch: NTSC's 7.5 IRE of setup and PAL's none are
// folded into `target` host-side, so this kernel only ever sees one number.
//
// Unpremultiplied. Mix 0 is the bit-exact identity, and so is a pixel already
// under the target — by construction rather than by short-circuit.

struct Params {
    // The largest Y + C the pixel may carry. Named `max_amp` and not `target`,
    // which the host side calls it: `target` is a WGSL reserved keyword, and a
    // module that uses one does not compile — silently, into a texture of zeros,
    // until something reads it.
    max_amp: f32,
    mode: u32,       // 0 reduce brightness, 1 reduce saturation, 2/3 key out
    mix_amt: f32,    // 0..1, blended against the unprocessed input
    _pad0: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

const LUMA = vec3<f32>(0.2126, 0.7152, 0.0722);

fn unpremult(c: vec4<f32>) -> vec3<f32> {
    if (c.a > 0.0) {
        return c.rgb / c.a;
    }
    return vec3<f32>(0.0);
}

// == cpu::broadcast_chroma.
fn chroma(v: vec3<f32>, y: f32) -> f32 {
    let cu = 0.493 * (v.b - y);
    let cv = 0.877 * (v.r - y);
    return sqrt(cu * cu + cv * cv);
}

@compute @workgroup_size(8, 8)
fn broadcast_safe(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let u = unpremult(o);
    let v = sqrt(max(u, vec3<f32>(0.0)));
    // Written out rather than `dot`, unlike every other kernel in the family:
    // two of the four modes turn this number into a *threshold on the alpha*, so
    // a fused multiply-add that only one path takes is a pixel keyed out on one
    // path and kept on the other (K-399). Three multiplies and two adds, in the
    // CPU reference's own order.
    let y = v.r * LUMA.r + v.g * LUMA.g + v.b * LUMA.b;
    let c = chroma(v, y);
    let amp = y + c;
    var sig = v;
    var out_a = o.a;
    if (p.mode == 0u) {
        // Scale the whole signal: Y and C are both linear in it, so the factor
        // that lands the amplitude on the target is exact.
        let k = min(p.max_amp / max(amp, 1e-6), 1.0);
        sig = v * k;
    } else if (p.mode == 1u) {
        // Pull toward the grey of the same luma: Y is unchanged and C scales.
        // A pixel whose luma alone is over the target ends fully desaturated and
        // still hot — §3.69 decision 3 says so rather than hiding it.
        let m = clamp(p.max_amp - y, 0.0, c) / max(c, 1e-6);
        sig = vec3<f32>(y) + (v - vec3<f32>(y)) * m;
    } else if (p.mode == 2u) {
        if (amp > p.max_amp) {
            out_a = 0.0;
        }
    } else {
        if (amp <= p.max_amp) {
            out_a = 0.0;
        }
    }
    let lit = sig * sig;
    let outv = o.rgb * (1.0 - p.mix_amt) + lit * out_a * p.mix_amt;
    let outa = o.a * (1.0 - p.mix_amt) + out_a * p.mix_amt;
    textureStore(dst, xy, vec4<f32>(outv, outa));
}

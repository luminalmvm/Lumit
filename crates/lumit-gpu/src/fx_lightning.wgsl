// Lightning (docs/08-EFFECTS.md §3.74): a forked bolt between two points.
// Mirrors lumit_core::fx::cpu::lightning and ::lightning_sample op-for-op (§1.6:
// the CPU is the oracle).
//
// THE BOLT IS NOT BUILT HERE. It arrives in the uniform as a list of straight
// segments, already displaced, already forked, already carrying each segment's
// fade (§3.74's first decision). That is what makes this kernel a plain minimum
// over capsules with no hashing at all — and it disposes of §1.6 for free, since
// both paths are handed the identical numbers rather than each generating them.
//
// The core and the glow are taken as a MAXIMUM over the segments and never a
// sum: every joint is shared by two segments and every fork meets its parent, so
// a sum would put a bright bead at each of them.
//
// Mix 0 is the bit-exact identity, and so is a bolt with no radius at all.

const MAX_SEGMENTS: u32 = 192u;

struct Params {
    core_colour: vec4<f32>,   // scene-linear; the alpha lane is ignored
    glow_colour: vec4<f32>,
    core_radius: f32,         // the filament's half-width, raster px
    glow_radius: f32,         // the halo's reach, raster px
    glow_opacity: f32,        // 0..1
    mix_amt: f32,             // 0..1, blended against the unprocessed input
    count: u32,               // how many segments are real
    composite: u32,           // 1 keeps the layer under the bolt, 0 replaces it
    matte_on: f32,            // 1 = the matte scales the bolt's opacity
    _pad1: u32,
    // (ax, ay, bx, by) in raster pixels.
    segs: array<vec4<f32>, 192>,
    // Four segments' fades to an element; index i is fades[i / 4][i % 4].
    fades: array<vec4<f32>, 48>,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// The Matte (docs/08 §2.6), bound for every kernel on this layout and
// read only under `matte_on` — bound to `src` when there is none, since a
// texture binding cannot be left empty.
@group(0) @binding(4) var matte: texture_2d<f32>;

// This pixel's matte strength (== cpu::matte_strength): premultiplied Rec. 709
// luma, clamped. The Channel pick and Invert already happened, once, at the
// seam (fx_matte_prepare.wgsl).
fn matte_k(xy: vec2<i32>) -> f32 {
    let m = textureLoad(matte, xy, 0);
    return clamp(m.r * 0.2126 + m.g * 0.7152 + m.b * 0.0722, 0.0, 1.0);
}

// A control pulled toward its neutral by k (== cpu::matte_toward), spelled out
// rather than `mix()` so that k = 1 is the value to the bit.
fn matte_toward(value: f32, neutral: f32, k: f32) -> f32 {
    return neutral * (1.0 - k) + value * k;
}

// == lumit_core::fx::cpu::segment_distance: the capsule, with the projection
// clamped so the ends are round rather than infinite.
fn seg_dist(pt: vec2<f32>, s: vec4<f32>) -> f32 {
    let d = s.zw - s.xy;
    let r = pt - s.xy;
    let t = clamp(dot(r, d) / max(dot(d, d), 1e-6), 0.0, 1.0);
    let o = r - t * d;
    return sqrt(dot(o, o));
}

@compute @workgroup_size(8, 8)
fn lightning(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let pt = vec2<f32>(f32(xy.x) + 0.5, f32(xy.y) + 0.5);
    var core = 0.0;
    var glow = 0.0;
    let n = min(p.count, MAX_SEGMENTS);
    for (var i = 0u; i < n; i = i + 1u) {
        let d = seg_dist(pt, p.segs[i]);
        let fade = p.fades[i / 4u][i % 4u];
        let c = clamp((p.core_radius + 0.5 - d) / max(p.core_radius, 0.5), 0.0, 1.0);
        core = max(core, fade * c);
        let g = clamp((p.glow_radius - d) / max(p.glow_radius, 1e-3), 0.0, 1.0);
        glow = max(glow, fade * g * g);
    }
    // The matte pulls the drawn bolt's opacity toward 0 per pixel, before the
    // composite: the core's own coverage and the Glow opacity together,
    // so the bolt fades along its length rather than the frame fading back.
    var glow_opacity = p.glow_opacity;
    if (p.matte_on != 0.0) {
        let k = matte_k(xy);
        core = matte_toward(core, 0.0, k);
        glow_opacity = matte_toward(glow_opacity, 0.0, k);
    }
    // The glow lights what the core has not already taken, so the two add to a
    // coverage that cannot exceed one.
    let gw = glow * glow_opacity * (1.0 - core);
    let cov = clamp(core + gw, 0.0, 1.0);
    let keep = (1.0 - cov) * f32(p.composite);
    let colour = p.core_colour.rgb * core + p.glow_colour.rgb * gw;
    let lit = vec4<f32>(o.rgb * keep + colour, o.a * keep + cov);
    textureStore(dst, xy, o * (1.0 - p.mix_amt) + lit * p.mix_amt);
}

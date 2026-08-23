// The shared path drawing (docs/08-EFFECTS.md §3.78 Scribble, §3.79 Stroke, and
// §3.76 Vegas' Mask/Path source). Mirrors lumit_core::fx::cpu::path_draw,
// ::path_draw_sample and ::path_draw_warp op-for-op (§1.6: the CPU is the
// oracle).
//
// ONE KERNEL, THREE EFFECTS. They differ entirely in where the line goes and
// hardly at all in how it is drawn, and where the line goes is decided on the
// CPU (K-408's polyline, then a hatch, a brush trail or the path itself). What
// arrives here is the same in all three cases: straight pieces in raster pixels,
// each carrying how far along the drawing its start sits.
//
// THE GEOMETRY IS NOT BUILT HERE, exactly as §3.74's bolt is not — which is what
// makes this a plain maximum over capsules, and what disposes of §1.6 for free,
// since both paths are handed identical numbers rather than each generating
// them.
//
// The coverage is a MAXIMUM over the pieces and never a sum: consecutive pieces
// share a joint, and a sum would put a bright bead at every one of them.
//
// A count of zero draws nothing, which is the documented no-op for an unset or
// deleted mask. Mix 0 and Opacity 0 are both the bit-exact identity.

const MAX_PIECES: u32 = 512u;

struct Params {
    colour: vec4<f32>,      // scene-linear; the alpha lane is ignored
    half_width: f32,        // half the drawn width, raster px
    band: f32,              // the soft edge either side of it, raster px
    inv_segment: f32,       // 1 / dash length; 0 for a continuous line
    duty: f32,              // the lit share of a dash; 2 == continuous
    phase: f32,             // the dash's phase, turns
    wiggle_amp: f32,        // how far the paper is displaced, raster px
    wiggle_freq: f32,       // cells per raster px
    wiggle_tick: f32,       // where in the wobble's evolution this frame sits
    opacity: f32,           // 0..1
    mix_amt: f32,           // 0..1, blended against the unprocessed input
    seed: u32,
    count: u32,             // how many pieces are real
    style: u32,             // 0 on original, 1 on transparent, 2 reveal original
    matte_on: f32,          // 1 = the matte scales Opacity per pixel (K-428)
    _pad1: u32,
    // (ax, ay, bx, by) in raster pixels.
    segs: array<vec4<f32>, 512>,
    // Four pieces' distances-along to an element; index i is arcs[i/4][i%4].
    arcs: array<vec4<f32>, 128>,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// The Matte (K-395, docs/08 §2.6), bound for every kernel on this layout and
// read only under `matte_on` — bound to `src` when there is none, since a
// texture binding cannot be left empty.
@group(0) @binding(4) var matte: texture_2d<f32>;

// This pixel's matte strength (== cpu::matte_strength): premultiplied Rec. 709
// luma, clamped. The Channel pick and Invert already happened, once, at the
// seam (fx_matte_prepare.wgsl, K-425).
fn matte_k(xy: vec2<i32>) -> f32 {
    let m = textureLoad(matte, xy, 0);
    return clamp(m.r * 0.2126 + m.g * 0.7152 + m.b * 0.0722, 0.0, 1.0);
}

// A control pulled toward its neutral by k (== cpu::matte_toward), spelled out
// rather than `mix()` so that k = 1 is the value to the bit.
fn matte_toward(value: f32, neutral: f32, k: f32) -> f32 {
    return neutral * (1.0 - k) + value * k;
}

// == lumit_core::fx::cpu::path_draw_warp. The scribble is not wobbled — the
// paper is, which costs one lookup a pixel instead of eight times the geometry.
fn warp(pt: vec2<f32>) -> vec2<f32> {
    if (p.wiggle_amp <= 0.0) {
        return pt;
    }
    let q = pt * p.wiggle_freq;
    let wx = nc_value3(p.seed, 0u, q.x, q.y, p.wiggle_tick, 0) * 2.0 - 1.0;
    let wy = nc_value3(p.seed, 1u, q.x, q.y, p.wiggle_tick, 0) * 2.0 - 1.0;
    return pt + vec2<f32>(wx, wy) * p.wiggle_amp;
}

// == lumit_core::fx::cpu::path_draw_sample.
fn coverage(pt: vec2<f32>) -> f32 {
    let q = warp(pt);
    var cov = 0.0;
    let n = min(p.count, MAX_PIECES);
    for (var i = 0u; i < n; i = i + 1u) {
        let s = p.segs[i];
        let d = s.zw - s.xy;
        let r = q - s.xy;
        let len2 = max(dot(d, d), 1e-6);
        let t = clamp(dot(r, d) / len2, 0.0, 1.0);
        let o = r - t * d;
        let dist = sqrt(dot(o, o));
        let across = clamp((p.half_width - dist) / p.band + 0.5, 0.0, 1.0);
        if (across <= 0.0) {
            continue;
        }
        // How far along the whole drawing this pixel's nearest point sits —
        // measured round the line, not projected across it.
        let arc = p.arcs[i / 4u][i % 4u] + t * sqrt(len2);
        let phase = arc * p.inv_segment + p.phase;
        let frac = phase - floor(phase);
        let soft = max(p.band * p.inv_segment, 1e-4);
        let along = clamp((p.duty - frac) / soft + 0.5, 0.0, 1.0);
        cov = max(cov, across * along);
    }
    return cov * p.opacity;
}

@compute @workgroup_size(8, 8)
fn path_draw(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    // `coverage` is the drawing's coverage times Opacity, and Opacity enters
    // nowhere else, so scaling its result IS scaling Opacity (K-428).
    var cov = coverage(vec2<f32>(f32(xy.x) + 0.5, f32(xy.y) + 0.5));
    if (p.matte_on != 0.0) {
        cov = matte_toward(cov, 0.0, matte_k(xy));
    }
    // Reveal original: the drawing is the matte, so colour and coverage alike
    // survive only where the brush went — which is what premultiplied means.
    if (p.style == 2u) {
        textureStore(dst, xy, o * (1.0 - p.mix_amt) + (o * cov) * p.mix_amt);
        return;
    }
    let keep = select(0.0, 1.0 - cov, p.style == 0u);
    let lit = vec4<f32>(o.rgb * keep + p.colour.rgb * cov, o.a * keep + cov);
    textureStore(dst, xy, o * (1.0 - p.mix_amt) + lit * p.mix_amt);
}

// Separable gaussian blur (docs/08-EFFECTS.md §3.8) — one pass per axis,
// direction in the uniform. Mirrors lumit_core::fx::cpu::blur_gaussian and
// ::blur_gaussian_matted op-for-op (§1.6: the CPU is the oracle): same
// σ = radius/2, same tap count ceil(radius), weights normalised over the FULL
// kernel regardless of edge policy, fixed tap order.
//
// **The Matte scales the radius, per pixel** (docs/08 §2.6). The
// Gaussian blur is one of the effects that claim the matte inside their own
// maths rather than taking the generic strength dissolve, and this is where:
// each destination pixel's own matte luma multiplies the radius before the
// kernel is built, so grey blurs narrowly and black not at all. That is a
// picture a dissolve cannot make — dissolving a 40 px blur to 50 % is a sharp
// image with a wide halo over it, not a 20 px blur.
//
// Both passes read the DESTINATION pixel's matte, which is what makes the two
// separable halves agree about how wide this pixel's kernel is.
//
// With `matte_on == 0` the radius and σ are used exactly as the host computed
// them, untouched — the byte-for-byte path every project saved before the
// Matte takes.

struct Params {
    dir: vec2<f32>,     // (1,0) horizontal pass, (0,1) vertical pass
    radius: f32,        // kernel half-width, px
    sigma: f32,         // radius * 0.5, clamped ≥ 1e-3
    edge: u32,          // 0 transparent, 1 repeat, 2 mirror
    mix_amt: f32,       // 0..1, blended against `orig` (1 on the h-pass)
    matte_on: f32,      // 1 = scale the radius by the matte's luma, 2 = by `field`
    _pad0: f32,
    // Camera depth of field, the layer seen from the eye with the view down +z:
    // [0].xyz and [1].xyz are one texel's step along x and y, [2].xyz is where
    // texel (0, 0)'s corner is, [0].w is the aperture over twice the focus
    // distance and [1].w is the focus distance.
    field: array<vec4<f32>, 3>,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;
// The generic Matte, bound for every kernel on this layout and read
// only by the ones that claim it. Bound to `src` when there is none, and gated
// by `matte_on` — a texture binding cannot be left empty.
@group(0) @binding(4) var matte: texture_2d<f32>;

// This pixel's matte strength: premultiplied Rec. 709 luma, clamped — the
// same reading `fx_matte_mix.wgsl` and cpu::matte_mix use, so "how much matte
// is here" means one thing across the whole campaign. The Channel pick and
// Invert have already happened, once, in fx_matte_prepare.wgsl.
fn matte_k(xy: vec2<i32>) -> f32 {
    let m = textureLoad(matte, xy, 0);
    return clamp(m.r * 0.2126 + m.g * 0.7152 + m.b * 0.0722, 0.0, 1.0);
}

// The camera's blur at this texel along this pass, in texels: half its circle
// of confusion on screen over how long one texel step along the pass is on
// screen, so the blur comes out round on screen however the layer is turned.
// Nothing behind the eye, and never past `radius`.
fn dof_radius(xy: vec2<i32>) -> f32 {
    let at = vec2<f32>(xy) + 0.5;
    let q = p.field[2].xyz + p.field[0].xyz * at.x + p.field[1].xyz * at.y;
    let step = select(p.field[1].xyz, p.field[0].xyz, p.dir.x > 0.5);
    let along = length(step.xy * q.z - q.xy * step.z);
    if (q.z <= 0.0 || along <= 0.0) {
        return 0.0;
    }
    return min(p.field[0].w * abs(q.z - p.field[1].w) * q.z / along, p.radius);
}

// Resolve a tap index under the edge policy; -1 means transparent (no tap).
fn edge_idx(i: i32, len: i32) -> i32 {
    if (i >= 0 && i < len) {
        return i;
    }
    if (p.edge == 1u) {
        return clamp(i, 0, len - 1);
    }
    if (p.edge == 2u) {
        var m = i;
        if (m < 0) {
            m = -m;
        } else {
            m = 2 * (len - 1) - m;
        }
        return clamp(m, 0, len - 1);
    }
    return -1;
}

// A workgroup's tile: TILE pixels along the pass, one a thread, on each of
// LINES neighbouring rows (or columns), every thread taking all of them. CHUNK
// is how much of the window sits in workgroup memory at once: 12 KiB of
// texels and 1.25 KiB of weights, inside the 16 KiB every device gives.
const TILE: i32 = 128;
const LINES: i32 = 4;
const CHUNK: i32 = 192;

var<workgroup> line: array<vec4<f32>, 768>; // LINES · CHUNK
// The offsets one chunk can hand the tile's pixels: CHUNK + TILE − 1.
var<workgroup> weight: array<f32, 319>;

// One pixel's blur, blended against the unprocessed input. A radius under
// zero is a pixel past the frame's edge.
fn finish(xy: vec2<i32>, r: i32, acc: vec4<f32>, wsum: f32) {
    if (r < 0) {
        return;
    }
    var v = acc / wsum;
    if (r == 0) {
        v = textureLoad(src, xy, 0);
    }
    textureStore(dst, xy, mix(textureLoad(orig, xy, 0), v, p.mix_amt));
}

// **Each texel fetched once a tile, not once a tap.** The tile's pixels read
// TILE + 2·reach texels a line between them, where reach is the uniform's
// radius: the matte and the camera only ever narrow it. That window goes
// through workgroup memory a chunk at a time, each texel resolved under the
// edge policy and fetched by one thread, and every pixel sums its own taps
// out of it. A radius of 470 read 941 texels a pixel a pass; now it reads
// about eight. On the vertical pass the four lines make each fetch and store
// four texels side by side, which is what the memory hands over in one go,
// and on both one weight serves four pixels. Each pixel's taps are still
// summed −r … +r with the same weights, so the picture is the one the direct
// loop made. Every slot of workgroup memory is written before it is read.
//
// The host dispatches (ceil(w / TILE), ceil(h / LINES)) for the horizontal
// pass and (ceil(w / LINES), ceil(h / TILE)) for the vertical one.
@compute @workgroup_size(128)
fn blur_pass(
    @builtin(workgroup_id) wg: vec3<u32>,
    @builtin(local_invocation_index) lid: u32,
) {
    let size = vec2<i32>(textureDimensions(src));
    let across_x = p.dir.x > 0.5;
    let axis_len = select(size.y, size.x, across_x);
    let t = i32(lid);
    let first = i32(select(wg.y, wg.x, across_x)) * TILE;
    let line0 = i32(select(wg.x, wg.y, across_x)) * LINES;
    var xy0 = vec2<i32>(line0, first + t);
    var next = vec2<i32>(1, 0);
    if (across_x) {
        xy0 = vec2<i32>(first + t, line0);
        next = vec2<i32>(0, 1);
    }

    var r = vec4<i32>(-1);
    var sig = vec4<f32>(p.sigma);
    for (var l = 0; l < LINES; l++) {
        let xy = xy0 + next * l;
        if (xy.x < size.x && xy.y < size.y) {
            var rad = p.radius;
            if (p.matte_on > 1.5) {
                rad = dof_radius(xy);
                sig[l] = max(rad * 0.5, 1e-3);
            } else if (p.matte_on != 0.0) {
                rad = p.radius * matte_k(xy);
                sig[l] = max(rad * 0.5, 1e-3);
            }
            r[l] = i32(ceil(rad));
        }
    }
    let r_most = max(max(r.x, r.y), max(r.z, r.w));

    // The same for the whole workgroup, so every thread meets every barrier.
    let reach = i32(ceil(p.radius));
    let span = TILE + 2 * reach;
    let fixed = p.matte_on == 0.0;
    var acc0 = vec4<f32>(0.0);
    var acc1 = vec4<f32>(0.0);
    var acc2 = vec4<f32>(0.0);
    var acc3 = vec4<f32>(0.0);
    var wsum = vec4<f32>(0.0);
    for (var c = 0; c < span; c += CHUNK) {
        let start = first - reach + c;
        for (var k = t; k < min(CHUNK, span - c); k += TILE) {
            let q = edge_idx(start + k, axis_len);
            for (var l = 0; l < LINES; l++) {
                var tap = vec2<i32>(line0 + l, q);
                if (across_x) {
                    tap = vec2<i32>(q, line0 + l);
                }
                var v = vec4<f32>(0.0);
                if (q >= 0 && tap.x < size.x && tap.y < size.y) {
                    v = textureLoad(src, tap, 0);
                }
                line[l * CHUNK + k] = v;
            }
        }
        // Without a matte every pixel takes the same weight at the same
        // offset, so each is worked out once: `weight[u]` is the offset
        // c − reach − (TILE − 1) + u, for the offsets a kernel can reach.
        if (fixed) {
            for (var u = t; u < CHUNK + TILE - 1; u += TILE) {
                let i = c - reach - (TILE - 1) + u;
                if (abs(i) <= reach) {
                    let d = f32(i) / max(p.sigma, 1e-3);
                    weight[u] = exp(-0.5 * d * d);
                }
            }
        }
        workgroupBarrier();
        // The taps inside the chunk: line[j] is offset j + c − reach − t.
        if (fixed) {
            let lo = max(0, t - c);
            let hi = min(CHUNK - 1, t + 2 * reach - c);
            for (var j = lo; j <= hi; j++) {
                let wt = weight[j - t + TILE - 1];
                wsum.x += wt;
                acc0 += line[j] * wt;
                acc1 += line[CHUNK + j] * wt;
                acc2 += line[2 * CHUNK + j] * wt;
                acc3 += line[3 * CHUNK + j] * wt;
            }
        } else {
            let lo = max(0, t + reach - c - r_most);
            let hi = min(CHUNK - 1, t + reach - c + r_most);
            let inv_sig = 1.0 / max(sig, vec4<f32>(1e-3));
            for (var j = lo; j <= hi; j++) {
                let i = j + c - reach - t;
                let d = vec4<f32>(f32(i)) * inv_sig;
                let wt = exp(-0.5 * d * d);
                let take = vec4<i32>(abs(i)) <= r;
                if (take.x) {
                    wsum.x += wt.x;
                    acc0 += line[j] * wt.x;
                }
                if (take.y) {
                    wsum.y += wt.y;
                    acc1 += line[CHUNK + j] * wt.y;
                }
                if (take.z) {
                    wsum.z += wt.z;
                    acc2 += line[2 * CHUNK + j] * wt.z;
                }
                if (take.w) {
                    wsum.w += wt.w;
                    acc3 += line[3 * CHUNK + j] * wt.w;
                }
            }
        }
        workgroupBarrier();
    }
    if (fixed) {
        wsum = vec4<f32>(wsum.x);
    }
    finish(xy0, r.x, acc0, wsum.x);
    finish(xy0 + next, r.y, acc1, wsum.y);
    finish(xy0 + next * 2, r.z, acc2, wsum.z);
    finish(xy0 + next * 3, r.w, acc3, wsum.w);
}

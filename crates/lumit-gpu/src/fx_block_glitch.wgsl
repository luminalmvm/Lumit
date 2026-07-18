// Block glitch — standalone block displacement (docs/08-EFFECTS.md §3.12,
// split out of the old combined Glitch effect by K-107: one of three
// now-separate one-thing effects, alongside Scanlines and Datamosh).
// Mirrors lumit_core::fx::cpu::block_glitch op-for-op (§1.6: the CPU is the
// oracle). The per-block hash runs here too, not host-precomputed: the
// block index is a per-pixel quantity (there are too many blocks to fit a
// table into the uniform), so `splitmix32` — a 32-bit sibling of Shake's
// splitmix64 lattice, added because WGSL has no 64-bit integer type to host
// the original — runs identical wrapping u32 ops on both sides.

struct Params {
    intensity: f32,
    seed: u32,
    tick: i32,
    block_size: f32,
    jitter_frac: f32,
    amount: f32,
    chan: f32,
    slice_frac: f32,
    mix_amt: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// A 32-bit avalanche mixer (== lumit_core::fx::splitmix32, identical
// wrapping u32 ops in the same order — exact on every GPU, so CPU and GPU
// agree on the integer hash bit-for-bit).
fn splitmix32(xin: u32) -> u32 {
    var x = xin;
    x = x + 0x9e3779b9u;
    x = x ^ (x >> 16u);
    x = x * 0x21f0aaadu;
    x = x ^ (x >> 15u);
    x = x * 0x735a2d97u;
    x = x ^ (x >> 15u);
    return x;
}

// == lumit_core::fx::block_hash01, same fold order; bitcast (not a value
// conversion) matches Rust's same-width `as u32` reinterpretation exactly.
fn block_hash01(channel: u32, bx: i32, by: i32) -> f32 {
    var h = p.seed;
    h = splitmix32(h ^ channel);
    h = splitmix32(h ^ bitcast<u32>(bx));
    h = splitmix32(h ^ bitcast<u32>(by));
    h = splitmix32(h ^ bitcast<u32>(p.tick));
    return f32(h >> 8u) / 16777216.0;
}

// Clamp-addressed bilinear sample at continuous pixel-centre coordinates
// (== cpu::bilinear, same arithmetic order).
fn bilinear(sx: f32, sy: f32, size: vec2<i32>) -> vec4<f32> {
    let fx = sx - 0.5;
    let fy = sy - 0.5;
    let x0 = floor(fx);
    let y0 = floor(fy);
    let tx = fx - x0;
    let ty = fy - y0;
    let x0i = i32(x0);
    let y0i = i32(y0);
    let c00 = textureLoad(
        src, vec2<i32>(clamp(x0i, 0, size.x - 1), clamp(y0i, 0, size.y - 1)), 0);
    let c10 = textureLoad(
        src, vec2<i32>(clamp(x0i + 1, 0, size.x - 1), clamp(y0i, 0, size.y - 1)), 0);
    let c01 = textureLoad(
        src, vec2<i32>(clamp(x0i, 0, size.x - 1), clamp(y0i + 1, 0, size.y - 1)), 0);
    let c11 = textureLoad(
        src, vec2<i32>(clamp(x0i + 1, 0, size.x - 1), clamp(y0i + 1, 0, size.y - 1)), 0);
    let top = c00 * (1.0 - tx) + c10 * tx;
    let bottom = c01 * (1.0 - tx) + c11 * tx;
    return top * (1.0 - ty) + bottom * ty;
}

@compute @workgroup_size(8, 8)
fn block_glitch(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(orig, xy, 0);
    // Neutral short-circuit (== the CPU reference's early return): holds
    // for any Mix, since it skips the blend entirely.
    if (p.intensity == 0.0) {
        textureStore(dst, xy, o);
        return;
    }

    let pos = vec2<f32>(xy) + vec2<f32>(0.5);
    let bw = max(p.block_size, 1.0);

    // Grid jitter (docs/08 §3.12 status note): a hashed offset of the
    // *nominal* block, scaled by Intensity, decides which block a pixel
    // reads.
    let bx0 = i32(floor(pos.x / bw));
    let by0 = i32(floor(pos.y / bw));
    let jx = (block_hash01(0u, bx0, by0) - 0.5) * 2.0 * p.jitter_frac * bw * p.intensity;
    let jy = (block_hash01(1u, bx0, by0) - 0.5) * 2.0 * p.jitter_frac * bw * p.intensity;
    let jpos = vec2<f32>(pos.x + jx, pos.y + jy);
    let bx = i32(floor(jpos.x / bw));
    let by = i32(floor(jpos.y / bw));

    let dx = (block_hash01(2u, bx, by) - 0.5) * 2.0 * p.amount * p.intensity;
    let dy = (block_hash01(3u, bx, by) - 0.5) * 2.0 * p.amount * p.intensity;
    let chan = (block_hash01(4u, bx, by) - 0.5) * 2.0 * p.chan * p.intensity;
    let slice_u = block_hash01(5u, bx, by);
    let slice_h_u = block_hash01(6u, bx, by);

    // Slice repeat: fold the block's own local Y to a short hashed repeat
    // height instead of a plain read.
    var eff_y = jpos.y;
    if (slice_u < p.slice_frac * p.intensity) {
        let local_y = jpos.y - f32(by) * bw;
        let repeat_h = max(slice_h_u * bw * 0.25, 1.0);
        let folded = local_y - floor(local_y / repeat_h) * repeat_h;
        eff_y = f32(by) * bw + folded;
    }
    let sx = jpos.x + dx;
    let sy = eff_y + dy;

    // R/B split from the block hash (alpha follows green, like RGB split).
    let r = bilinear(sx - chan, sy, size).r;
    let g = bilinear(sx, sy, size);
    let b = bilinear(sx + chan, sy, size).b;
    let c = vec4<f32>(r, g.g, b, g.a);

    textureStore(dst, xy, mix(o, c, p.mix_amt));
}

// Mosaic (docs/08-EFFECTS.md §3.65): the frame in flat blocks. Mirrors
// lumit_core::fx::cpu::mosaic op-for-op (§1.6: the CPU is the oracle).
//
// EVERY BLOCK BOUNDARY IS AN INTEGER DIVISION, deliberately. A block edge
// decided by floor(x / block_width) in floating point puts a pixel in different
// blocks on the two paths wherever the division comes out exact — K-399's rule
// about a threshold, arriving on a coordinate — and integer division has no such
// tie. The stratified sample positions are integers for the same reason.
//
// The averaged mode reads at most 8x8 positions of the block rather than all of
// it: a true mean of a block of a 1080p frame at the default grid is thousands
// of taps redone by every pixel inside it. A block under eight pixels across is
// sampled completely, so a fine mosaic IS an exact mean (§3.65 note 2).
//
// Premultiplied: averaging premultiplied colour is what compositing means, and
// the alpha is blocked with it. Mix 0 is the bit-exact identity.

struct Params {
    blocks_x: i32,
    blocks_y: i32,
    sharp: f32,      // 1 = the block's centre pixel, 0 = the sampled mean
    mix_amt: f32,    // 0..1, blended against the unprocessed input
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// == cpu::MOSAIC_MAX_SAMPLES.
const MAX_SAMPLES: i32 = 8;

// == cpu::mosaic_span.
fn span_lo(x: i32, len: i32, blocks: i32) -> i32 {
    let i = (x * blocks) / len;
    return (i * len) / blocks;
}

// == cpu::mosaic_span's upper bound.
fn span_hi(x: i32, len: i32, blocks: i32) -> i32 {
    let i = (x * blocks) / len;
    return ((i + 1) * len) / blocks;
}

// == cpu::mosaic_sample.
fn sample_at(lo: i32, span: i32, n: i32, k: i32) -> i32 {
    return lo + (2 * k * span + span) / (2 * n);
}

fn tap(x: i32, y: i32, size: vec2<i32>) -> vec4<f32> {
    let c = clamp(vec2<i32>(x, y), vec2<i32>(0, 0), size - vec2<i32>(1, 1));
    return textureLoad(src, c, 0);
}

@compute @workgroup_size(8, 8)
fn mosaic(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let bx = clamp(p.blocks_x, 1, 2000);
    let by = clamp(p.blocks_y, 1, 2000);
    let x0 = span_lo(xy.x, size.x, bx);
    let x1 = span_hi(xy.x, size.x, bx);
    let y0 = span_lo(xy.y, size.y, by);
    let y1 = span_hi(xy.y, size.y, by);
    var v = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    if (p.sharp > 0.5) {
        v = tap(x0 + (x1 - x0) / 2, y0 + (y1 - y0) / 2, size);
    } else {
        let nx = clamp(x1 - x0, 1, MAX_SAMPLES);
        let ny = clamp(y1 - y0, 1, MAX_SAMPLES);
        var acc = vec4<f32>(0.0, 0.0, 0.0, 0.0);
        for (var j = 0; j < ny; j++) {
            let sy = sample_at(y0, y1 - y0, ny, j);
            for (var i = 0; i < nx; i++) {
                let sx = sample_at(x0, x1 - x0, nx, i);
                acc = acc + tap(sx, sy, size);
            }
        }
        v = acc * (1.0 / f32(nx * ny));
    }
    textureStore(dst, xy, o * (1.0 - p.mix_amt) + v * p.mix_amt);
}

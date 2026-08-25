// Lens flare ghost blur (docs/08 §3.27, K-261, K-263): one direction of a
// separable box blur over the flare buffer — FlareSim's Ghost Blur, run
// horizontal + vertical × 3 passes to approximate a Gaussian. A touch of
// out-of-focus softness that also hides quad-grid facets at low qualities.
//
// The sum is taken through a workgroup line cache (K-263). Read straight from
// the texture, an 80 px radius costs 161 fetches per pixel per pass and six
// passes run — near a thousand fetches a pixel, which is what made a high
// Ghost softness on a large frame a stall rather than a slow frame. A
// workgroup covers 64 consecutive pixels along the blur axis, so the 64 + 2r
// texels they need between them are fetched ONCE into shared memory and every
// thread sums out of that: about 3.5 fetches per pixel instead of 161. The
// summation order is unchanged (d = −r … +r over the same clamped
// coordinates), so the result is bit-for-bit what the naive loop produced.

struct BlurParams {
    w: u32,
    h: u32,
    radius: u32,
    dir: u32, // 0 horizontal, 1 vertical
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(2) var<uniform> bp: BlurParams;

// Pixels one workgroup writes, and the widest radius the CPU side allows
// (`MAX_BLUR_RADIUS_PX`) — together they size the cache.
const TILE: u32 = 64u;
const MAX_R: u32 = 80u;
const CACHE: u32 = 224u; // TILE + 2 · MAX_R

var<workgroup> line: array<vec4<f32>, CACHE>;

// The texel at position `n` along the blur axis of the row/column `across`,
// clamped to the frame exactly as the direct loop clamped it.
fn sample_at(n: i32, across: i32, len: i32) -> vec4<f32> {
    let c = clamp(n, 0, len - 1);
    var xy = vec2<i32>(c, across);
    if (bp.dir != 0u) {
        xy = vec2<i32>(across, c);
    }
    return textureLoad(src, xy, 0);
}

// `gid.x` runs ALONG the blur axis and `gid.y` across it — so for the
// vertical pass the caller dispatches (ceil(h/64), w, 1), not (w, h, 1).
@compute @workgroup_size(64)
fn blur(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(workgroup_id) wg: vec3<u32>,
    @builtin(local_invocation_index) lid: u32,
) {
    let r = min(bp.radius, MAX_R);
    var len = i32(bp.w);
    var across_len = i32(bp.h);
    if (bp.dir != 0u) {
        len = i32(bp.h);
        across_len = i32(bp.w);
    }
    let across = i32(gid.y);
    // The first cached texel: the tile's first pixel minus the radius.
    let base = i32(wg.x * TILE) - i32(r);
    let needed = TILE + 2u * r;
    // Four strided loads cover the widest cache; a thread whose slot is past
    // what this radius needs simply does not load. Out-of-range rows still
    // take part — the barrier below is uniform control flow.
    for (var i = lid; i < needed; i = i + TILE) {
        var v = vec4<f32>(0.0);
        if (across < across_len) {
            v = sample_at(base + i32(i), across, len);
        }
        line[i] = v;
    }
    workgroupBarrier();
    if (gid.x >= u32(len) || across >= across_len) {
        return;
    }
    var acc = vec4<f32>(0.0);
    for (var k = 0u; k <= 2u * r; k = k + 1u) {
        acc = acc + line[lid + k];
    }
    let norm = 1.0 / f32(2u * r + 1u);
    var xy = vec2<i32>(i32(gid.x), across);
    if (bp.dir != 0u) {
        xy = vec2<i32>(across, i32(gid.x));
    }
    textureStore(dst, xy, acc * norm);
}

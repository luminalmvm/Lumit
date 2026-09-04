// Dominant-motion tile reduction for Fast motion blur
// (docs/impl/optical-flow.md §4.5 item 3). The CPU twin is
// `lumit_core::fx::cpu::motion_blur_tiles`, op-for-op: one thread per tile,
// the same pixels scanned in the same raster order, the same strictly-greater
// comparison — so ties land on the same pixel and the winning vector is a copy
// rather than an average, which makes the two bit-identical rather than close.
//
// The score is confidence-weighted length, not raw length: a badly matched patch
// can produce one wild long vector, and the weighting stops it capturing the tile
// away from a slower vector the measurement actually believes. That matters
// because the blur kernel hands uncertain pixels this vector to borrow.
//
// The weight is floored at SCORE_FLOOR rather than running to zero. A region
// where nothing matched — smoke, a muzzle flash, fast water — would otherwise
// score zero everywhere and read as still, handing a zero direction to exactly
// the pixels that most need a borrowed one.
//
// In: the flow field (rgba32float, .xy motion, .z confidence). Out: one texel
// per tile, `(u, v, score, 0)`, also rgba32float — the vectors must stay exact
// f32 to match an f32 oracle.

const SCORE_FLOOR: f32 = 0.25; // lumit_core::fx::cpu::MB_SCORE_FLOOR

// Scalars, not a vec3 tail: a vec3<i32> aligns to 16 bytes and would silently
// make this struct 32 bytes against the host's 16.
struct Params {
    tile: i32,          // MB_TILE: tile side in pixels
    vector_scale: f32,  // px@raster a full Motion vectors channel means
    pad1: i32,
    pad2: i32,
};

@group(0) @binding(0) var flow: texture_2d<f32>;
@group(0) @binding(1) var tiles: texture_storage_2d<rgba32float, write>;
@group(0) @binding(2) var<uniform> p: Params;

// A supplied **Motion vectors** layer read as a flow field (docs/08
// §3.2) — the twin of lumit_core::fx::cpu::motion_vectors_field. Red is
// sideways, green is up-and-down, mid-grey is standing still, and
// `vector_scale` says how many pixels a full channel means. Confidence is 1
// everywhere: a supplied vector is not a measurement that can have failed.
//
// It shares this file's layout (a picture in, an rgba32float field out, one
// uniform) so it needs no seam of its own: `flow` is the vectors layer here
// and `tiles` is the field. Everything downstream — the reduction above, the
// blur itself — then reads one kind of field and knows nothing about where it
// came from.
@compute @workgroup_size(8, 8)
fn mb_vectors(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(tiles));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let m = textureLoad(flow, xy, 0);
    textureStore(tiles, xy, vec4<f32>(
        (m.r - 0.5) * p.vector_scale,
        (m.g - 0.5) * p.vector_scale,
        1.0,
        0.0,
    ));
}

@compute @workgroup_size(8, 8)
fn mb_tilemax(@builtin(global_invocation_id) gid: vec3<u32>) {
    let tdim = vec2<i32>(textureDimensions(tiles));
    let t = vec2<i32>(gid.xy);
    if (t.x >= tdim.x || t.y >= tdim.y) {
        return;
    }
    let size = vec2<i32>(textureDimensions(flow));
    var best = vec3<f32>(0.0);
    var best_score = -1.0;
    let y1 = min((t.y + 1) * p.tile, size.y);
    let x1 = min((t.x + 1) * p.tile, size.x);
    for (var py = t.y * p.tile; py < y1; py++) {
        for (var px = t.x * p.tile; px < x1; px++) {
            let f = textureLoad(flow, vec2<i32>(px, py), 0);
            let trust = SCORE_FLOOR + (1.0 - SCORE_FLOOR) * clamp(f.z, 0.0, 1.0);
            let score = trust * sqrt(f.x * f.x + f.y * f.y);
            if (score > best_score) {
                best_score = score;
                best = vec3<f32>(f.x, f.y, score);
            }
        }
    }
    textureStore(tiles, t, vec4<f32>(best, 0.0));
}

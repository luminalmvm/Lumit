// One garbage mask against the screen matte (docs/08-EFFECTS.md §3.21, K-546).
// Mirrors lumit_core::fx::cpu::mask_fill_at op-for-op (§1.6: the CPU is the
// oracle).
//
// A garbage mask is a shape drawn round the part of the shot the keyer should
// not be trusted with. The shape arrives as GEOMETRY -- a closed outline in
// raster pixels, built host-side by `cpu::mask_fill_params` -- not as a picture
// of itself, so there is no extra texture and both render paths walk identical
// numbers.
//
// The kernel runs once per mask: mode 0 is the INSIDE hold-out (its inside is
// forced opaque, a max) and mode 1 the OUTSIDE one (its inside is forced
// transparent, a min against the complement). A count of zero is never
// dispatched at all -- that is the row's documented no-op.
//
// Inside or outside is an even-odd crossing count; how soft the edge is comes
// from the mask's own feather, read as a ramp across the signed distance to the
// nearest piece of the outline. That is the identical reading the mask's own
// coverage is given, so a hold-out and the mask it was drawn from soften alike.
// The sign flips exactly where the distance is zero and the ramp reads 0.5 there
// from either side, so the result is continuous in position despite the hard
// branch, and the ULP oracle holds.

const MAX_PIECES: u32 = 512u;

struct Params {
    count: u32,       // how many pieces are real
    ramp: f32,        // the feather width, raster px, never below one
    expansion: f32,   // the mask's own grow (+) / shrink (-), raster px
    mode: u32,        // 0 = inside (force opaque), 1 = outside (force clear)
    // (ax, ay, bx, by) in raster pixels, closed.
    segments: array<vec4<f32>, 512>,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// How much of this point the mask covers, 0..1 (== cpu::mask_fill_at).
fn cov(px: f32, py: f32) -> f32 {
    // Not `inf`: WGSL has no infinity literal, and the CPU reference starts its
    // reduction from this same number.
    var d2 = 1e30;
    var crossings = 0;
    for (var i: u32 = 0u; i < p.count && i < MAX_PIECES; i = i + 1u) {
        let s = p.segments[i];
        let ax = s.x;
        let ay = s.y;
        let bx = s.z;
        let by = s.w;
        let ex = bx - ax;
        let ey = by - ay;
        let len2 = ex * ex + ey * ey;
        var t = 0.0;
        if (len2 > 0.0) {
            t = clamp(((px - ax) * ex + (py - ay) * ey) / len2, 0.0, 1.0);
        }
        let qx = ax + ex * t - px;
        let qy = ay + ey * t - py;
        d2 = min(d2, qx * qx + qy * qy);
        // The half-open rule (`<=` one end, `>` the other) counts a vertex on
        // the ray once, never twice -- and guarantees `by != ay`, so the divide
        // below is safe.
        if ((ay <= py) != (by <= py)) {
            let x = ax + (py - ay) / (by - ay) * (bx - ax);
            if (x > px) {
                crossings = crossings + 1;
            }
        }
    }
    var sgn = -1.0;
    if ((crossings & 1) == 1) {
        sgn = 1.0;
    }
    let d = sqrt(d2) * sgn;
    return clamp(0.5 + (d + p.expansion) / p.ramp, 0.0, 1.0);
}

// STAGE 6 (K-546). Pixel centres, the same coordinate the mask rasteriser
// samples on, so a hold-out lands where its mask is drawn.
@compute @workgroup_size(8, 8)
fn matte_mask(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let m0 = textureLoad(src, xy, 0).r;
    let c = cov(f32(xy.x) + 0.5, f32(xy.y) + 0.5);
    var v = max(m0, c);
    if (p.mode == 1u) {
        v = min(m0, 1.0 - c);
    }
    textureStore(dst, xy, vec4<f32>(v));
}

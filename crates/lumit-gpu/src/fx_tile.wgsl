// Tile (docs/08-EFFECTS.md §3.39): one rectangle of the picture stamped across
// the frame. Mirrors lumit_core::fx::cpu::tile_into op-for-op (§1.6: the CPU is
// the oracle).
//
// The four per cents arrive as FRACTIONS of the raster rather than lengths,
// because the kernel already knows the raster and the host does not. Outside the
// output window the result is transparent; inside, the pixel's position within
// its own tile picks the sample, mirrored on odd tile indices when Mirror edges
// is on. Mix 0 is the bit-exact identity, and so are the shipped defaults —
// a whole-frame tile cut from the frame's middle with no phase (K-542), which
// this kernel answers by copying rather than by resampling, because the divide
// and the multiply that undo one another do not always do so in fp32.
//
// **The destination may be BIGGER than the source** (K-542). Output width and
// height above 100 % stamp copies past the frame's edges, so the host allocates
// `out_size` (cpu::tile_raster) and the frame sits in the middle of it; every
// coordinate below is in the incoming frame's own pixels, which is why `origin`
// comes off first.

struct Params {
    centre_tile: vec4<f32>,   // xy = tile centre (raster px), zw = tile size as a frame fraction
    output_frac: vec2<f32>,   // output window as a frame fraction
    phase: f32,               // Phase ÷ 360, in tiles
    mix_amt: f32,             // 0..1, blended against the unprocessed input
    mirror_edges: u32,
    horizontal_phase_shift: u32,
    out_size: vec2<u32>,      // the destination raster, >= the source (K-542)
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// == cpu::bilinear_edge with the Repeat policy: the sample always lies inside
// the stamped rectangle, which may itself hang off the frame.
fn tap(x: i32, y: i32, size: vec2<i32>) -> vec4<f32> {
    return textureLoad(src, vec2<i32>(clamp(x, 0, size.x - 1), clamp(y, 0, size.y - 1)), 0);
}

fn bilinear_repeat(sx: f32, sy: f32, size: vec2<i32>) -> vec4<f32> {
    let fx = sx - 0.5;
    let fy = sy - 0.5;
    let x0 = floor(fx);
    let y0 = floor(fy);
    let tx = fx - x0;
    let ty = fy - y0;
    let x0i = i32(x0);
    let y0i = i32(y0);
    let c00 = tap(x0i, y0i, size);
    let c10 = tap(x0i + 1, y0i, size);
    let c01 = tap(x0i, y0i + 1, size);
    let c11 = tap(x0i + 1, y0i + 1, size);
    let top = c00 * (1.0 - tx) + c10 * tx;
    let bottom = c01 * (1.0 - tx) + c11 * tx;
    return top * (1.0 - ty) + bottom * ty;
}

@compute @workgroup_size(8, 8)
fn tile(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let out_size = vec2<i32>(p.out_size);
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= out_size.x || xy.y >= out_size.y) {
        return;
    }
    let fw = f32(size.x);
    let fh = f32(size.y);
    let cx = fw * 0.5;
    let cy = fh * 0.5;

    // == the identity short-circuit cpu::tile_into takes, same comparisons.
    if (all(p.centre_tile.zw == vec2<f32>(1.0, 1.0))
        && all(p.output_frac == vec2<f32>(1.0, 1.0))
        && p.phase == 0.0
        && all(p.centre_tile.xy == vec2<f32>(cx, cy))) {
        textureStore(dst, xy, textureLoad(src, xy, 0));
        return;
    }

    // Where the incoming frame's top-left sits in the destination. Whole pixels
    // deliberately: a pixel that passes through must be the same pixel.
    let origin = (out_size - size) / 2;
    let sxy = xy - origin;
    let tw = max(fw * p.centre_tile.z, 1e-3);
    let th = max(fh * p.centre_tile.w, 1e-3);
    let half_w = fw * p.output_frac.x * 0.5;
    let half_h = fh * p.output_frac.y * 0.5;
    let px = f32(sxy.x) + 0.5;
    let py = f32(sxy.y) + 0.5;

    // What was here before the effect: the frame's own pixel inside it, and
    // nothing at all in the margin the growth added. The coordinate is clamped
    // and the value chosen afterwards — an early return before textureLoad lets
    // the compiler hoist an out-of-range fetch, which this backend answers with
    // a live alpha lane (the §3.53 hazard).
    let inside = all(sxy >= vec2<i32>(0)) && all(sxy < size);
    let o = select(vec4<f32>(0.0),
                   textureLoad(src, clamp(sxy, vec2<i32>(0), size - 1), 0),
                   inside);

    var v = vec4<f32>(0.0);
    if (abs(px - cx) <= half_w && abs(py - cy) <= half_h) {
        var u = (px - p.centre_tile.x) / tw + 0.5;
        var t = (py - p.centre_tile.y) / th + 0.5;
        // The phase shift is applied along one axis using the OTHER axis's whole
        // tile index, so the two floors are taken in the order the switch picks.
        var iu: f32;
        var it: f32;
        if (p.horizontal_phase_shift != 0u) {
            iu = floor(u);
            t = t + iu * p.phase;
            it = floor(t);
        } else {
            it = floor(t);
            u = u + it * p.phase;
            iu = floor(u);
        }
        var fu = u - iu;
        var ft = t - it;
        if (p.mirror_edges != 0u) {
            // Two's complement `& 1` is odd-ness for negative indices too.
            if ((i32(iu) & 1) != 0) { fu = 1.0 - fu; }
            if ((i32(it) & 1) != 0) { ft = 1.0 - ft; }
        }
        v = bilinear_repeat(p.centre_tile.x + (fu - 0.5) * tw,
                            p.centre_tile.y + (ft - 0.5) * th,
                            size);
    }
    textureStore(dst, xy, o * (1.0 - p.mix_amt) + v * p.mix_amt);
}

// Spherize (docs/08-EFFECTS.md §3.52): a glass ball held over the picture.
// Mirrors lumit_core::fx::cpu::spherize op-for-op (§1.6: the CPU is the oracle).
//
// The two directions are mutually inverse radial maps rather than one map with a
// sign — (2/pi)*asin(rho) magnifies the middle and sin(rho*pi/2) is exactly its
// undo — so a bulge and a pinch of the same strength, radius and centre cancel.
//
// One arc sine or sine a pixel — §3.42's fourth note.
//
// Mix 0, Bulge 0 and Radius 0 are all the bit-exact identity.

struct Params {
    centre: vec2<f32>,   // raster pixels
    radius: f32,         // raster pixels
    inv_radius: f32,     // 1 / radius, floored host-side
    bulge: f32,          // -1..1; the sign chooses the map, the magnitude blends
    mix_amt: f32,        // 0..1, blended against the unprocessed input
    matte_on: f32,       // 1 = the matte scales Bulge
    _pad1: f32,
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

const PI: f32 = 3.1415927;

// == cpu::bilinear_edge with the Transparent policy (edge == 0).
// The tap NEVER loads out of bounds. A guard that early-returns before the
// textureLoad reads correctly on paper, but the load is side-effect-free and
// gets hoisted above the branch; on at least one Windows backend the hoisted
// out-of-range fetch comes back with a live alpha lane, so a pixel whose four
// taps are all outside the frame arrives opaque-and-wrong instead of empty.
// Clamping the coordinate and choosing the value afterwards has no such hazard,
// and costs one `select`. (Found by the §1.6 oracle for Polar coordinates,
// docs/08 §3.50 — the first kernel in the batch whose samples leave the frame.)
fn tap(x: i32, y: i32, size: vec2<i32>) -> vec4<f32> {
    let inside = x >= 0 && x < size.x && y >= 0 && y < size.y;
    let c = clamp(vec2<i32>(x, y), vec2<i32>(0, 0), size - vec2<i32>(1, 1));
    return select(vec4<f32>(0.0, 0.0, 0.0, 0.0), textureLoad(src, c, 0), inside);
}

fn bilinear_transparent(sx: f32, sy: f32, size: vec2<i32>) -> vec4<f32> {
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
fn spherize(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let px = f32(xy.x) + 0.5;
    let py = f32(xy.y) + 0.5;
    let dx = px - p.centre.x;
    let dy = py - p.centre.y;
    let r = sqrt(dx * dx + dy * dy);
    var sx = px;
    var sy = py;
    // The matte scales Bulge per pixel (== cpu::spherize_matted).
    var bulge = p.bulge;
    if (p.matte_on != 0.0) {
        bulge = bulge * matte_k(xy);
    }
    // Bulge 0 short-circuits (see cpu::spherize): the blend would leave `scale`
    // at rho / rho, and this backend's reciprocal-multiply division answers a
    // hair under 1 — a whole picture of resampling for an effect turned off.
    if (r < p.radius && r > 0.0 && bulge != 0.0) {
        // Clamped: a radius rounded a hair below r would hand asin an argument
        // above 1 and it would answer NaN.
        let rho = min(r * p.inv_radius, 1.0);
        var mapped = sin(rho * PI * 0.5);
        if (bulge >= 0.0) {
            mapped = (2.0 / PI) * asin(rho);
        }
        let scale = (rho + (mapped - rho) * abs(bulge)) / rho;
        sx = p.centre.x + dx * scale;
        sy = p.centre.y + dy * scale;
    }
    let v = bilinear_transparent(sx, sy, size);
    textureStore(dst, xy, o * (1.0 - p.mix_amt) + v * p.mix_amt);
}

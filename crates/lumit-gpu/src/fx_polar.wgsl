// Polar coordinates (docs/08-EFFECTS.md §3.50): the frame bent into a circle,
// and back. Mirrors lumit_core::fx::cpu::polar_coordinates op-for-op (§1.6: the
// CPU is the oracle).
//
// The centre and the radius scale are functions of the raster, which this kernel
// knows and the host does not, so neither is in the uniform (§3.39's precedent).
// The radius spans half the frame diagonal, so the frame's corners lie inside
// the mapped disc; the angle starts straight up and turns clockwise, which is
// also where a wrapped picture's seam falls.
//
// Three transcendentals a pixel — §3.42's fourth note and K-399's rule: the
// angle IS a function of the pixel and cannot be lifted host-side, so both paths
// run their own platform's atan2/sin/cos and the oracle is judged on absolute
// difference over a smooth corpus.
//
// Mix 0 and Interpolation 0 are both the bit-exact identity.

struct Params {
    interp: f32,      // Interpolation / 100
    mix_amt: f32,     // 0..1, blended against the unprocessed input
    to_polar: u32,    // 1 = Rectangular to polar, 0 = its exact inverse
    _pad0: u32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

const TAU: f32 = 6.2831855;

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
fn polar_coordinates(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let fw = f32(size.x);
    let fh = f32(size.y);
    let cx = fw * 0.5;
    let cy = fh * 0.5;
    let radius = 0.5 * sqrt(fw * fw + fh * fh);
    let px = f32(xy.x) + 0.5;
    let py = f32(xy.y) + 0.5;
    var qx: f32;
    var qy: f32;
    if (p.to_polar != 0u) {
        let dx = px - cx;
        let dy = py - cy;
        // atan2(x, -y) is "from straight up, clockwise" on a raster whose y
        // grows downward — §3.46 and §3.47's reading.
        let turns = atan2(dx, -dy) / TAU;
        qx = (turns - floor(turns)) * fw;
        qy = sqrt(dx * dx + dy * dy) / radius * fh;
    } else {
        let theta = px / fw * TAU;
        let r = py / fh * radius;
        qx = cx + r * sin(theta);
        qy = cy - r * cos(theta);
    }
    let v = bilinear_transparent(
        px + (qx - px) * p.interp,
        py + (qy - py) * p.interp,
        size,
    );
    textureStore(dst, xy, o * (1.0 - p.mix_amt) + v * p.mix_amt);
}

// Twirl (docs/08-EFFECTS.md §3.51): the picture wrung round a point. Mirrors
// lumit_core::fx::cpu::twirl op-for-op (§1.6: the CPU is the oracle).
//
// The falloff is squared, so the twist eases out with zero slope at the rim
// rather than stopping at a crease. A rotation about the centre preserves the
// radius, so a twirl never samples outside its own circle; the only samples that
// leave the frame are the ones whose circle already hung over the edge, and
// those read transparent.
//
// One sine and cosine a pixel — §3.42's fourth note and K-399's rule: the angle
// is a function of the radius and cannot be lifted host-side.
//
// Mix 0, Angle 0 and Radius 0 are all the bit-exact identity.

struct Params {
    centre: vec2<f32>,   // raster pixels
    radius: f32,         // raster pixels
    inv_radius: f32,     // 1 / radius, floored host-side
    angle: f32,          // radians; positive turns the picture clockwise
    mix_amt: f32,        // 0..1, blended against the unprocessed input
    _pad0: f32,
    _pad1: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

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
fn twirl(@builtin(global_invocation_id) gid: vec3<u32>) {
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
    if (r < p.radius) {
        let t = 1.0 - r * p.inv_radius;
        let phi = p.angle * t * t;
        let s = sin(phi);
        let c = cos(phi);
        // R(-phi) applied to the offset: the picture turns by +phi.
        sx = p.centre.x + dx * c + dy * s;
        sy = p.centre.y - dx * s + dy * c;
    }
    let v = bilinear_transparent(sx, sy, size);
    textureStore(dst, xy, o * (1.0 - p.mix_amt) + v * p.mix_amt);
}

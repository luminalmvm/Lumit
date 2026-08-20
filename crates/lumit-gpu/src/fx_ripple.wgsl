// Ripple (docs/08-EFFECTS.md §3.53): rings spreading from a point.
// Mirrors lumit_core::fx::cpu::ripple op-for-op (§1.6: the CPU is the oracle).
//
// The envelope 27/4 * rho * (1 - rho)^2 is zero at the centre as well as at the
// rim, which removes the direction singularity at r = 0 exactly, and its
// constant makes Wave height literally the farthest a pixel moves.
//
// One sine and cosine a pixel — §3.42's fourth note and K-399's rule.
//
// Mix 0, Radius 0 and Wave height 0 are all the bit-exact identity.

struct Params {
    centre: vec2<f32>,   // raster pixels
    radius: f32,         // raster pixels
    inv_radius: f32,     // 1 / radius, floored host-side
    amount: f32,         // raster pixels: Wave height times 27/4
    inv_width: f32,      // 1 / Wave width, raster pixels
    turns: f32,          // Evolution / 360, in whole waves
    mix_amt: f32,        // 0..1, blended against the unprocessed input
    asymmetric: u32,     // 1 = add the tangential half of the wave
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

const TAU: f32 = 6.2831855;

// == cpu::bilinear_edge with the Repeat policy (edge == 1): a ring wider than
// the frame's own half-height reaches outside it, and a transparent edge would
// punch a bite out of the picture where the crest was.
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
fn ripple(@builtin(global_invocation_id) gid: vec3<u32>) {
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
    if (r < p.radius && r > 0.0 && p.amount != 0.0) {
        let rho = min(r * p.inv_radius, 1.0);
        let one = 1.0 - rho;
        let env = rho * one * one * p.amount;
        let phase = TAU * (r * p.inv_width - p.turns);
        let sn = sin(phase);
        let cs = cos(phase);
        let inv_r = 1.0 / r;
        let nx = dx * inv_r;
        let ny = dy * inv_r;
        if (p.asymmetric != 0u) {
            sx = px + (nx * sn - ny * cs) * env;
            sy = py + (ny * sn + nx * cs) * env;
        } else {
            sx = px + nx * sn * env;
            sy = py + ny * sn * env;
        }
    }
    let v = bilinear_repeat(sx, sy, size);
    textureStore(dst, xy, o * (1.0 - p.mix_amt) + v * p.mix_amt);
}

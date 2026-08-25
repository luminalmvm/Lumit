// Bezier warp (docs/08-EFFECTS.md §3.55): the frame's four edges bent.
// Mirrors lumit_core::fx::cpu::bezier_warp op-for-op (§1.6: the CPU is the
// oracle).
//
// The twelve points bend the frame's four edges into cubic Beziers and the
// inside is the Coons patch they bound. Rendering asks "where did this output
// pixel come from", so every pixel SOLVES the patch backwards by Newton's method
// from its own position — which is the identity patch's own answer, so an
// untouched frame converges before it starts.
//
// Outside the patch is transparent. A sample landing within a thousandth of a
// pixel of its own centre is snapped to it, so an unbent region of a bent frame
// is bit-exact rather than resampled (§3.55 decision 4).

struct Params {
    // The twelve points in AE's clockwise walk from the upper left, two to a
    // vec4: corner, two handles, corner, two handles, …
    q0: vec4<f32>,
    q1: vec4<f32>,
    q2: vec4<f32>,
    q3: vec4<f32>,
    q4: vec4<f32>,
    q5: vec4<f32>,
    mix_amt: f32,        // 0..1, blended against the unprocessed input
    steps: u32,          // Newton steps a pixel, 1..12
    matte_on: f32,       // 1 = the matte scales the bend from the straight frame (K-427)
    _pad1: u32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// The Matte (K-395, docs/08 §2.6), bound for every kernel on this layout and
// read only under `matte_on` — bound to `src` when there is none, since a
// texture binding cannot be left empty.
@group(0) @binding(4) var matte: texture_2d<f32>;

// This pixel's matte strength (== cpu::matte_strength): premultiplied Rec. 709
// luma, clamped. The Channel pick and Invert already happened, once, at the
// seam (fx_matte_prepare.wgsl, K-425).
fn matte_k(xy: vec2<i32>) -> f32 {
    let m = textureLoad(matte, xy, 0);
    return clamp(m.r * 0.2126 + m.g * 0.7152 + m.b * 0.0722, 0.0, 1.0);
}

// A control pulled toward its neutral by k (== cpu::matte_toward), spelled out
// rather than `mix()` so that k = 1 is the value to the bit.
fn matte_toward(value: f32, neutral: f32, k: f32) -> f32 {
    return neutral * (1.0 - k) + value * k;
}

// == cpu::BEZ_MIN_DET, cpu::BEZ_MAX_RESIDUAL_PX and cpu::BEZ_SNAP_PX.
const MIN_DET: f32 = 1e-9;
// A Newton solve has to be CHECKED, not trusted: outside the patch there is no
// answer, and an unchecked iteration wanders until it happens to land in 0..1,
// which scatters stray pixels across the empty part of the frame.
const MAX_RESIDUAL_PX: f32 = 1.0;
const SNAP_PX: f32 = 1e-3;

// == cpu::bilinear_edge with the Transparent policy (edge == 0); the tap never
// loads out of bounds (docs/08 §3.50's note).
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

// == cpu::bez: one cubic Bezier and its derivative, packed as (pos, tangent).
fn bez(a: vec2<f32>, b: vec2<f32>, c: vec2<f32>, d: vec2<f32>, t: f32) -> vec4<f32> {
    let s = 1.0 - t;
    let w0 = s * s * s;
    let w1 = 3.0 * s * s * t;
    let w2 = 3.0 * s * t * t;
    let w3 = t * t * t;
    let g0 = 3.0 * s * s;
    let g1 = 6.0 * s * t;
    let g2 = 3.0 * t * t;
    let pos = a * w0 + b * w1 + c * w2 + d * w3;
    let tan = (b - a) * g0 + (c - b) * g1 + (d - c) * g2;
    return vec4<f32>(pos, tan);
}

struct Patch {
    s: vec2<f32>,
    su: vec2<f32>,
    sv: vec2<f32>,
};

// == cpu::coons: the two boundary curves in each direction blended across, minus
// the bilinear surface on the four corners that the two blends count twice.
fn coons(u: f32, v: f32) -> Patch {
    let ul = p.q0.xy;
    let ur = p.q1.zw;
    let lr = p.q3.xy;
    let ll = p.q4.zw;
    let top = bez(ul, p.q0.zw, p.q1.xy, ur, u);
    let bot = bez(ll, p.q4.xy, p.q3.zw, lr, u);
    let lef = bez(ul, p.q5.zw, p.q5.xy, ll, v);
    let rig = bez(ur, p.q2.xy, p.q2.zw, lr, v);
    let corners = (1.0 - u) * (1.0 - v) * ul
                + u * (1.0 - v) * ur
                + (1.0 - u) * v * ll
                + u * v * lr;
    var out: Patch;
    out.s = (1.0 - v) * top.xy + v * bot.xy + (1.0 - u) * lef.xy + u * rig.xy - corners;
    out.su = (1.0 - v) * top.zw + v * bot.zw - lef.xy + rig.xy
           - (-(1.0 - v) * ul + (1.0 - v) * ur - v * ll + v * lr);
    out.sv = -top.xy + bot.xy + (1.0 - u) * lef.zw + u * rig.zw
           - (-(1.0 - u) * ul - u * ur + (1.0 - u) * ll + u * lr);
    return out;
}

@compute @workgroup_size(8, 8)
fn bezier_warp(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let fw = f32(size.x);
    let fh = f32(size.y);
    let px = f32(xy.x) + 0.5;
    let py = f32(xy.y) + 0.5;
    var u = px / fw;
    var v = py / fh;
    for (var i = 0u; i < p.steps; i = i + 1u) {
        let pt = coons(u, v);
        let fx = pt.s.x - px;
        let fy = pt.s.y - py;
        let det = pt.su.x * pt.sv.y - pt.su.y * pt.sv.x;
        if (abs(det) < MIN_DET) {
            break;
        }
        let inv = 1.0 / det;
        u = u - (fx * pt.sv.y - fy * pt.sv.x) * inv;
        v = v - (pt.su.x * fy - pt.su.y * fx) * inv;
    }
    // The solve, verified: in range *and* actually solving.
    let back = coons(u, v);
    let miss = max(abs(back.s.x - px), abs(back.s.y - py));
    var val = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    if (u >= 0.0 && u <= 1.0 && v >= 0.0 && v <= 1.0 && miss <= MAX_RESIDUAL_PX) {
        var sx = u * fw;
        var sy = v * fh;
        if (abs(sx - px) < SNAP_PX && abs(sy - py) < SNAP_PX) {
            sx = px;
            sy = py;
        }
        // The matte scales the displacement toward none, after the snap and
        // read at the destination pixel (K-427, == cpu::bezier_warp_matted).
        if (p.matte_on != 0.0) {
            let k = matte_k(xy);
            sx = matte_toward(sx, px, k);
            sy = matte_toward(sy, py, k);
        }
        val = bilinear_transparent(sx, sy, size);
    }
    textureStore(dst, xy, o * (1.0 - p.mix_amt) + val * p.mix_amt);
}

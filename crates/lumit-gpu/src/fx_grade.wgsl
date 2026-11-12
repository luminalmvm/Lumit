// CDL and the colour warper: two grade effects beside the Correction, each
// one pass. Mirrors lumit_core::fx::effects::cdl and colour_warper, reading
// the words their packed forms wrote (the CPU is the oracle).

struct Params {
    w: array<vec4<f32>, 16>,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

const LUMA = vec3<f32>(0.2126, 0.7152, 0.0722);
const BOUND: f32 = 65504.0;
const RED_ANGLE: f32 = 1.7960518;
const LOG_TOP: f32 = 1.4679963;
const TAU: f32 = 6.2831855;

fn sgnpow(x: f32, e: f32) -> f32 {
    if (x == 0.0) {
        return 0.0;
    }
    if (x > 0.0) {
        return pow(x, e);
    }
    return -pow(-x, e);
}

// == correction::maths::encode and decode.
fn encode(x: f32, space: i32) -> f32 {
    if (space == 0) {
        return sgnpow(x, 1.0 / 2.4);
    }
    if (space == 1) {
        if (x <= 0.0078125) {
            return 10.540237 * x + 0.07290553;
        }
        return (log2(x) + 9.72) / 17.52;
    }
    return x;
}

fn decode(y: f32, space: i32) -> f32 {
    if (space == 0) {
        return sgnpow(y, 2.4);
    }
    if (space == 1) {
        if (y <= 0.15525114) {
            return (y - 0.07290553) / 10.540237;
        }
        return exp2(min(y, LOG_TOP) * 17.52 - 9.72);
    }
    return y;
}

fn luma(v: vec3<f32>) -> f32 {
    return v.r * LUMA.r + v.g * LUMA.g + v.b * LUMA.b;
}

fn v_in(o: vec4<f32>, space: i32) -> vec3<f32> {
    var u = vec3<f32>(0.0);
    if (o.a > 0.0) {
        u = o.rgb / o.a;
    }
    return vec3<f32>(encode(u.r, space), encode(u.g, space), encode(u.b, space));
}

// == CdlPacked::pixel.
@compute @workgroup_size(8, 8)
fn cdl(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let space = i32(p.w[0].x);
    let sat = p.w[0].y;
    let mix_amt = p.w[0].z;
    let v = v_in(o, space);
    var x: vec3<f32>;
    for (var c = 0; c < 3; c = c + 1) {
        let s = clamp(v[c] * p.w[1][c] + p.w[2][c], 0.0, 1.0);
        x[c] = select(0.0, pow(s, p.w[3][c]), s > 0.0);
    }
    let y = luma(x);
    var out: vec3<f32>;
    for (var c = 0; c < 3; c = c + 1) {
        let g = clamp(y + sat * (x[c] - y), 0.0, 1.0);
        let m = v[c] * (1.0 - mix_amt) + g * mix_amt;
        out[c] = decode(m, space) * o.a;
    }
    textureStore(dst, xy, vec4<f32>(out, o.a));
}

fn ycc(v: vec3<f32>) -> vec3<f32> {
    let y = luma(v);
    return vec3<f32>(y, (v.b - y) / 1.8556, (v.r - y) / 1.5748);
}

fn rgb_of(c: vec3<f32>) -> vec3<f32> {
    let r = c.x + 1.5748 * c.z;
    let b = c.x + 1.8556 * c.y;
    let g = (c.x - LUMA.r * r - LUMA.b * b) / LUMA.g;
    return vec3<f32>(r, g, b);
}

fn hue_of(cb: f32, cr: f32) -> f32 {
    if (cb * cb + cr * cr < 1e-12) {
        return 0.0;
    }
    let t = (atan2(cr, cb) - RED_ANGLE) / TAU;
    return t - floor(t);
}

// Mesh point i's move.
fn mesh(i: i32) -> vec2<f32> {
    let w = p.w[1 + i / 2];
    if (i % 2 == 0) {
        return w.xy;
    }
    return w.zw;
}

fn spline(a: vec2<f32>, b: vec2<f32>, c: vec2<f32>, d: vec2<f32>, t: f32) -> vec2<f32> {
    let t2 = t * t;
    let t3 = t * t * t;
    return 0.5 * (2.0 * b
        + (c - a) * t
        + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2
        + (3.0 * b - a - 3.0 * c + d) * t3);
}

// == WarpPacked::ring.
fn ring(r: i32, theta: f32) -> vec2<f32> {
    if (r == 0) {
        return mesh(0);
    }
    let n = i32(p.w[0].x);
    let k0 = floor(theta);
    let t = theta - k0;
    let k = i32(k0);
    var s: array<vec2<f32>, 4>;
    for (var o = 0; o < 4; o = o + 1) {
        let spoke = ((k + o - 1) % n + n) % n;
        s[o] = mesh(1 + (r - 1) * n + spoke);
    }
    return spline(s[0], s[1], s[2], s[3], t);
}

fn ring_ext(r: i32, theta: f32) -> vec2<f32> {
    if (r < 0) {
        return 2.0 * ring(0, theta) - ring(1, theta);
    }
    if (r > 3) {
        return 2.0 * ring(3, theta) - ring(2, theta);
    }
    return ring(r, theta);
}

// == WarpPacked::pixel.
@compute @workgroup_size(8, 8)
fn colour_warper(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let v = v_in(o, 0);
    let c = ycc(v);
    let pp = vec2<f32>(c.y / 0.5, c.z / 0.5);
    let theta = p.w[0].x * hue_of(c.y, c.z);
    let rho = 3.0 * sqrt(pp.x * pp.x + pp.y * pp.y);
    let f = clamp(rho, 0.0, 3.0);
    let r0 = min(i32(floor(f)), 2);
    let t = f - f32(r0);
    let d = spline(
        ring_ext(r0 - 1, theta),
        ring_ext(r0, theta),
        ring_ext(r0 + 1, theta),
        ring_ext(r0 + 2, theta),
        t,
    );
    var w = rgb_of(vec3<f32>(c.x, 0.5 * (pp.x + d.x), 0.5 * (pp.y + d.y)));
    if (p.w[0].y == 0.0) {
        let top = max(max(v.r, v.g), v.b);
        let now = max(max(w.r, w.g), w.b);
        if (top > 0.0 && now > 1e-6) {
            w = w * (top / now);
        }
    }
    let mix_amt = p.w[0].z;
    var out: vec3<f32>;
    for (var i = 0; i < 3; i = i + 1) {
        let m = v[i] * (1.0 - mix_amt) + w[i] * mix_amt;
        out[i] = clamp(decode(m, 0), -BOUND, BOUND) * o.a;
    }
    textureStore(dst, xy, vec4<f32>(out, o.a));
}

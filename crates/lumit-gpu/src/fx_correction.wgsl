// Correction: one whole grade node. Mirrors
// lumit_core::fx::effects::correction::maths function for function, and reads
// the numbers `Packed` made (the CPU is the oracle). The formulas are written
// out at the top of maths.rs.
//
// Four entry points, run only when wanted:
//   correction_vin      the picture in the grading space, for the blurs
//   correction_qualify  the qualifier's key, before Grow and Blur
//   correction_key      the node's key: windows x qualifier x matte
//   correction_grade    the grade, mixed back by the key
//   correction_parallel a Parallel node joined to its run: src + (a - b)

struct Win {
    a: vec4<f32>,  // shape, centre x, centre y, opacity
    b: vec4<f32>,  // half width, half height, cos, sin
    c: vec4<f32>,  // corner radius, outer softness, inner softness, 1 invert + 2 subtract
    d: vec4<f32>,  // outline bounds: min x, min y, max x, max y
    e: vec4<f32>,  // outline start, outline count
};

struct Params {
    g0: vec4<f32>,         // space, mix, lum mix, key on
    pre_gain: vec4<f32>,   // rgb; w = midtone detail on
    offset: vec4<f32>,
    lift: vec4<f32>,
    gamma: vec4<f32>,      // the exponents 2^(-2 gamma)
    gain: vec4<f32>,
    shadow: vec4<f32>,
    midtone: vec4<f32>,
    highlight: vec4<f32>,
    g1: vec4<f32>,         // low range, high range, contrast, pivot
    g2: vec4<f32>,         // saturation, hue cos, hue sin, boost
    g3: vec4<f32>,         // shadows, highlights, detail, low knee
    g4: vec4<f32>,         // high knee, curves on, hue curves on, mixer on
    mix_r: vec4<f32>,      // w = keep luma
    mix_g: vec4<f32>,
    mix_b: vec4<f32>,
    q0: vec4<f32>,         // mode, hue, hue below, hue above
    q1: vec4<f32>,         // hue softness, use hue, use sat, use lum
    q2: vec4<f32>,         // sat low, high, softness; w = denoised
    q3: vec4<f32>,         // lum low, high, softness; w = invert
    q4: vec4<f32>,         // rgb low; w = softness
    q5: vec4<f32>,         // rgb high
    q6: vec4<f32>,         // clean low, clean high, clip low, clip high
    k0: vec4<f32>,         // qualifier on, matte on, windows on
    win: array<Win, 4>,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var tex_a: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;
@group(0) @binding(4) var tex_b: texture_2d<f32>;
// The curve tables (257 entries each, ten of them), then the outlines.
@group(0) @binding(6) var<storage, read> data: array<f32>;

const LUMA = vec3<f32>(0.2126, 0.7152, 0.0722);
const BOUND: f32 = 65504.0;
const RED_ANGLE: f32 = 1.7960518;
const LOG_TOP: f32 = 1.4679963;
const TAU: f32 = 6.2831855;
const TABLE: i32 = 257;
const POLY_AT: u32 = 2570u;

fn sgnpow(x: f32, e: f32) -> f32 {
    if (x == 0.0) {
        return 0.0;
    }
    if (x > 0.0) {
        return pow(x, e);
    }
    return -pow(-x, e);
}

fn ss(e0: f32, e1: f32, x: f32) -> f32 {
    if (e1 > e0) {
        let t = clamp((x - e0) / (e1 - e0), 0.0, 1.0);
        return t * t * (3.0 - 2.0 * t);
    }
    if (x < e0) {
        return 0.0;
    }
    return 1.0;
}

fn held(v: vec3<f32>) -> vec3<f32> {
    return clamp(v, vec3<f32>(-BOUND), vec3<f32>(BOUND));
}

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

fn chroma(v: vec3<f32>) -> f32 {
    return max(max(v.r, v.g), v.b) - min(min(v.r, v.g), v.b);
}

fn turn(cb: f32, cr: f32, rot: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(cb * rot.x - cr * rot.y, cb * rot.y + cr * rot.x);
}

fn band(x: f32, lo: f32, hi: f32, soft: f32) -> f32 {
    var low = 1.0;
    if (lo > 0.0) {
        low = ss(lo - soft, lo, x);
    }
    var high = 1.0;
    if (hi < 1.0) {
        high = 1.0 - ss(hi, hi + soft, x);
    }
    return low * high;
}

fn entry(t: i32, i: i32) -> f32 {
    return data[t * TABLE + i];
}

// == maths::table_at, which reads a table the way cpu::curve_at does.
fn table_at(t: i32, x: f32) -> f32 {
    let last = f32(TABLE - 1);
    let s = x * last;
    let fi = floor(clamp(s, 0.0, last - 1.0));
    let i = min(i32(fi), TABLE - 2);
    let f = s - fi;
    let a = entry(t, i);
    return a + (entry(t, i + 1) - a) * f;
}

fn qualify(v: vec3<f32>) -> f32 {
    let mode = i32(p.q0.x);
    var k = 1.0;
    if (mode == 1) {
        let c = ycc(v);
        if (p.q1.y != 0.0) {
            let h = hue_of(c.y, c.z);
            let t = h - p.q0.y + 0.5;
            let d = t - floor(t) - 0.5;
            if (d >= 0.0) {
                k = k * (1.0 - ss(p.q0.w, p.q0.w + p.q1.x, d));
            } else {
                k = k * (1.0 - ss(p.q0.z, p.q0.z + p.q1.x, -d));
            }
        }
        if (p.q1.z != 0.0) {
            k = k * band(chroma(v), p.q2.x, p.q2.y, p.q2.z);
        }
        if (p.q1.w != 0.0) {
            k = k * band(c.x, p.q3.x, p.q3.y, p.q3.z);
        }
    } else if (mode == 2) {
        k = band(v.r, p.q4.x, p.q5.x, p.q4.w)
            * band(v.g, p.q4.y, p.q5.y, p.q4.w)
            * band(v.b, p.q4.z, p.q5.z, p.q4.w);
    } else if (mode == 3) {
        k = band(luma(v), p.q3.x, p.q3.y, p.q3.z);
    }
    k = clamp((k - p.q6.x) / (p.q6.y - p.q6.x), 0.0, 1.0);
    return min(max(k, p.q6.z), p.q6.w);
}

fn vertex(w: Win, i: u32) -> vec2<f32> {
    let k = POLY_AT + 2u * (u32(w.e.x) + i);
    return vec2<f32>(data[k], data[k + 1u]);
}

fn polygon(q: vec2<f32>, w: Win) -> f32 {
    let n = u32(w.e.y);
    var best = 3.4e38;
    var sgn = 1.0;
    var j = n - 1u;
    for (var i = 0u; i < n; i = i + 1u) {
        let vi = vertex(w, i);
        let vj = vertex(w, j);
        let e = vj - vi;
        let d = q - vi;
        let ee = max(dot(e, e), 1e-12);
        let t = clamp(dot(d, e) / ee, 0.0, 1.0);
        let b = d - e * t;
        best = min(best, dot(b, b));
        let c0 = q.y >= vi.y;
        let c1 = q.y < vj.y;
        let c2 = e.x * d.y > e.y * d.x;
        if ((c0 && c1 && c2) || (!c0 && !c1 && !c2)) {
            sgn = -sgn;
        }
        j = i;
    }
    return sgn * sqrt(best);
}

fn shape_distance(q: vec2<f32>, w: Win) -> f32 {
    let shape = i32(w.a.x);
    if (shape == 1) {
        let r = max(w.b.xy, vec2<f32>(1e-3));
        let a = q / r;
        let b = a / r;
        let k0 = length(a);
        let k1 = length(b);
        if (k1 < 1e-12) {
            return -min(r.x, r.y);
        }
        return k0 * (k0 - 1.0) / k1;
    }
    if (shape == 2) {
        let r = w.c.x;
        let d = abs(q) - w.b.xy + vec2<f32>(r);
        return length(max(d, vec2<f32>(0.0))) + min(max(d.x, d.y), 0.0) - r;
    }
    if (shape == 3) {
        let bx = vec2<f32>(max(w.d.x - q.x, q.x - w.d.z), max(w.d.y - q.y, q.y - w.d.w));
        if (max(bx.x, bx.y) > w.c.y) {
            return max(bx.x, bx.y);
        }
        return polygon(q, w);
    }
    return q.y;
}

fn window_weight(px: vec2<f32>, w: Win) -> f32 {
    let dx = px.x - w.a.y;
    let dy = px.y - w.a.z;
    let q = vec2<f32>(w.b.z * dx + w.b.w * dy, w.b.z * dy - w.b.w * dx);
    let d = shape_distance(q, w);
    var k = 1.0 - ss(-w.c.z, w.c.y, d);
    if ((u32(w.c.w) & 1u) != 0u) {
        k = 1.0 - k;
    }
    return k * w.a.w;
}

fn windows(px: vec2<f32>) -> f32 {
    var acc = -1.0;
    for (var i = 0; i < 4; i = i + 1) {
        let w = p.win[i];
        if (w.a.x == 0.0) {
            continue;
        }
        let subtract = (u32(w.c.w) & 2u) != 0u;
        if (acc < 0.0) {
            acc = select(0.0, 1.0, subtract);
        }
        let k = window_weight(px, w);
        if (subtract) {
            acc = acc * (1.0 - k);
        } else {
            acc = max(acc, k);
        }
    }
    if (acc < 0.0) {
        return 1.0;
    }
    return acc;
}

fn unpremult(o: vec4<f32>) -> vec3<f32> {
    if (o.a > 0.0) {
        return o.rgb / o.a;
    }
    return vec3<f32>(0.0);
}

fn v_in(o: vec4<f32>, space: i32) -> vec3<f32> {
    let u = unpremult(o);
    return vec3<f32>(encode(u.r, space), encode(u.g, space), encode(u.b, space));
}

fn grade(o: vec4<f32>, key: f32, detail: f32) -> vec4<f32> {
    let space = i32(p.g0.x);
    let u = unpremult(o);
    let vin = vec3<f32>(encode(u.r, space), encode(u.g, space), encode(u.b, space));
    let ug = u * p.pre_gain.rgb;
    var v = vec3<f32>(encode(ug.r, space), encode(ug.g, space), encode(ug.b, space));

    // Wheels.
    let y0 = luma(v);
    var x = v + p.offset.rgb;
    x = x + p.lift.rgb * (vec3<f32>(1.0) - x);
    v = vec3<f32>(
        sgnpow(x.r, p.gamma.r) * p.gain.r,
        sgnpow(x.g, p.gamma.g) * p.gain.g,
        sgnpow(x.b, p.gamma.b) * p.gain.b,
    );
    let yw = luma(v);
    let s = ss(p.g1.x - 0.1, p.g1.x + 0.1, yw);
    let h = ss(p.g1.y - 0.1, p.g1.y + 0.1, yw);
    v = v + (1.0 - s) * p.shadow.rgb + (s - h) * p.midtone.rgb + h * p.highlight.rgb;
    v = held(v);
    v = v + vec3<f32>((1.0 - p.g0.z) * (y0 - luma(v)));

    // Adjust.
    v = (v - vec3<f32>(p.g1.w)) * p.g1.z + vec3<f32>(p.g1.w);
    let yt = luma(v);
    let tonal = p.g3.x * (1.0 - ss(0.0, 0.5, yt)) + p.g3.y * ss(0.5, 1.0, yt);
    v = v + vec3<f32>(tonal);
    if (p.g4.w != 0.0) {
        let before = luma(v);
        v = vec3<f32>(
            p.mix_r.x * v.r + p.mix_r.y * v.g + p.mix_r.z * v.b,
            p.mix_g.x * v.r + p.mix_g.y * v.g + p.mix_g.z * v.b,
            p.mix_b.x * v.r + p.mix_b.y * v.g + p.mix_b.z * v.b,
        );
        if (p.mix_r.w != 0.0) {
            v = v + vec3<f32>(before - luma(v));
        }
    }
    let sat = clamp(chroma(v), 0.0, 1.0);
    let c = ycc(v);
    let f = p.g2.x * (1.0 + p.g2.w * (1.0 - sat));
    let t = turn(c.y, c.z, p.g2.yz);
    v = rgb_of(vec3<f32>(c.x, t.x * f, t.y * f));
    if (p.g3.z != 0.0) {
        let yd = luma(v);
        let lift = p.g3.z * (luma(vin) - detail) * clamp(4.0 * yd * (1.0 - yd), 0.0, 1.0);
        v = v + vec3<f32>(lift);
    }

    // Curves.
    if (p.g4.y != 0.0) {
        v = vec3<f32>(
            table_at(0, table_at(1, v.r)),
            table_at(0, table_at(2, v.g)),
            table_at(0, table_at(3, v.b)),
        );
    }
    let kl = p.g3.w;
    let kh = p.g4.x;
    for (var ch = 0; ch < 3; ch = ch + 1) {
        var e = v[ch];
        if (kh < 1.0 && e > kh) {
            let tt = (e - kh) / (1.0 - kh);
            e = kh + (1.0 - kh) * tt / (1.0 + tt);
        }
        if (kl > 0.0 && e < kl) {
            let tt = (kl - e) / kl;
            e = kl - kl * tt / (1.0 + tt);
        }
        v[ch] = e;
    }
    if (p.g4.z != 0.0) {
        let cc = ycc(v);
        let hue = hue_of(cc.y, cc.z);
        let sn = clamp(chroma(v), 0.0, 1.0);
        let yl = clamp(cc.x, 0.0, 1.0);
        let spin = (table_at(4, hue) - 0.5) * TAU;
        let ff = 2.0 * table_at(5, hue) * 2.0 * table_at(7, yl) * 2.0 * table_at(8, sn);
        let dy = 0.5 * sn * (table_at(6, hue) - 0.5 + table_at(9, sn) - 0.5);
        let tt = turn(cc.y, cc.z, vec2<f32>(cos(spin), sin(spin)));
        v = rgb_of(vec3<f32>(cc.x + dy, tt.x * ff, tt.y * ff));
    }
    v = held(v);

    // Out.
    let k = key * p.g0.y;
    let wv = vin * (1.0 - k) + v * k;
    let lin = vec3<f32>(decode(wv.r, space), decode(wv.g, space), decode(wv.b, space));
    return vec4<f32>(held(lin) * o.a, o.a);
}

@compute @workgroup_size(8, 8)
fn correction_vin(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    textureStore(dst, xy, vec4<f32>(v_in(o, i32(p.g0.x)), o.a));
}

@compute @workgroup_size(8, 8)
fn correction_qualify(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    var v: vec3<f32>;
    if (p.q2.w != 0.0) {
        v = textureLoad(tex_a, xy, 0).rgb;
    } else {
        v = v_in(textureLoad(src, xy, 0), i32(p.g0.x));
    }
    textureStore(dst, xy, vec4<f32>(qualify(v)));
}

@compute @workgroup_size(8, 8)
fn correction_key(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    var k = windows(vec2<f32>(xy) + vec2<f32>(0.5));
    if (p.k0.x != 0.0) {
        let q = textureLoad(tex_a, xy, 0).r;
        if (p.q3.w != 0.0) {
            k = k * (1.0 - q);
        } else {
            k = k * q;
        }
    }
    if (p.k0.y != 0.0) {
        k = k * clamp(luma(textureLoad(tex_b, xy, 0).rgb), 0.0, 1.0);
    }
    textureStore(dst, xy, vec4<f32>(k));
}

@compute @workgroup_size(8, 8)
fn correction_grade(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    var key = 1.0;
    if (p.g0.w != 0.0) {
        key = textureLoad(tex_a, xy, 0).r;
    }
    var detail = 0.0;
    if (p.pre_gain.w != 0.0) {
        detail = luma(textureLoad(tex_b, xy, 0).rgb);
    }
    textureStore(dst, xy, grade(o, key, detail));
}

// What a Parallel node changed (tex_a, drawn from the run's input tex_b),
// added to what the run made (src). combine::parallel's twin.
@compute @workgroup_size(8, 8)
fn correction_parallel(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let acc = textureLoad(src, xy, 0);
    textureStore(dst, xy, acc + (textureLoad(tex_a, xy, 0) - textureLoad(tex_b, xy, 0)));
}

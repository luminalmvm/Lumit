struct Params {
    source: vec2<f32>,
    threshold: f32,
    colourise: u32,
    mask: vec2<f32>,
    edge: f32,
    based_on: u32,
    gain: f32,
    source_opacity: f32,
    rays_opacity: f32,
    blend: u32,
    mix_amt: f32,
    scale: f32,
    count: u32,
    total: f32,
    weights: array<vec4<f32>, 2>,
    stops: array<vec4<f32>, 7>,
    lengths: array<vec4<f32>, 256>,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;
@group(0) @binding(4) var matte: texture_2d<f32>;

const TAU: f32 = 6.28318530718;
const TABLE: i32 = 1024;
const TAPS: i32 = 8;

fn tap(x: i32, y: i32, size: vec2<i32>) -> vec4<f32> {
    let c = clamp(vec2<i32>(x, y), vec2<i32>(0, 0), size - vec2<i32>(1, 1));
    let outside = x < 0 || y < 0 || x >= size.x || y >= size.y;
    return select(textureLoad(src, c, 0), vec4<f32>(0.0), outside);
}

fn bilinear(sx: f32, sy: f32, size: vec2<i32>) -> vec4<f32> {
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

fn table(i: i32) -> f32 {
    return p.lengths[i / 4][i % 4];
}

fn length_at(d: vec2<f32>) -> f32 {
    if (d.x == 0.0 && d.y == 0.0) { return table(0); }
    let turn = atan2(d.y, d.x) / TAU;
    let x = (turn - floor(turn)) * f32(TABLE);
    let i = min(i32(floor(x)), TABLE - 1);
    let f = x - floor(x);
    let a = table(i);
    let b = table((i + 1) % TABLE);
    return a + (b - a) * f;
}

fn lightness(c: vec3<f32>) -> f32 {
    return 0.5 * (max(max(c.r, c.g), c.b) + min(min(c.r, c.g), c.b));
}

@compute @workgroup_size(8, 8)
fn lr_light(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let c = textureLoad(src, xy, 0);
    let pos = vec2<f32>(xy) + vec2<f32>(0.5);
    var k = 1.0;
    if (p.threshold > 0.0) {
        let l = pow(max(lightness(c.rgb), 0.0), 1.0 / 2.2);
        k = max(l - p.threshold, 0.0) / max(l, 1e-5);
    }
    if (p.mask.x >= 0.0) {
        let d = length(pos - p.source);
        if (p.mask.y > 0.0) {
            let t = clamp((d - p.mask.x) / p.mask.y, 0.0, 1.0);
            k = k * (1.0 - t * t * (3.0 - 2.0 * t));
        } else if (d > p.mask.x) {
            k = 0.0;
        }
    }
    if (p.colourise == 0u) {
        textureStore(dst, xy, c * k);
        return;
    }
    let a = c.a;
    var u = vec3<f32>(0.0);
    if (a > 0.0) {
        u = c.rgb / a;
    }
    var v = 0.0;
    var alpha = a;
    switch p.based_on {
        case 0u: {
            v = lightness(u) * a;
        }
        case 1u: {
            v = (0.2126 * u.r + 0.7152 * u.g + 0.0722 * u.b) * a;
        }
        case 2u: {
            v = a;
        }
        case 3u: {
            var low = a;
            for (var t = 0; t < 8; t++) {
                let angle = f32(t) * 0.785398163397;
                let at = bilinear(pos.x + p.edge * cos(angle), pos.y + p.edge * sin(angle), size);
                low = min(low, at.a);
            }
            v = clamp(a - low, 0.0, 1.0);
            alpha = v;
        }
        case 4u: {
            v = u.r * a;
        }
        case 5u: {
            v = u.g * a;
        }
        default: {
            v = u.b * a;
        }
    }
    v = v * k;
    textureStore(dst, xy, vec4<f32>(v, v, v, alpha * k));
}

@compute @workgroup_size(8, 8)
fn lr_gather(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let pos = vec2<f32>(xy) + vec2<f32>(0.5);
    let d = pos - p.source;
    let step = length_at(d) * p.scale;
    var sum = vec4<f32>(0.0);
    for (var j = 0; j < TAPS; j++) {
        let s = exp(-f32(j) * step);
        let at = bilinear(p.source.x + d.x * s, p.source.y + d.y * s, size);
        sum += at * p.weights[j / 4][j % 4];
    }
    textureStore(dst, xy, sum / p.total);
}

fn gradient(t: f32) -> vec3<f32> {
    let n = i32(p.count);
    for (var s = 1; s < n; s++) {
        if (t <= p.stops[s].w) {
            let span = p.stops[s].w - p.stops[s - 1].w;
            var f = 1.0;
            if (span > 0.0) {
                f = clamp((t - p.stops[s - 1].w) / span, 0.0, 1.0);
            }
            let a = p.stops[s - 1].rgb;
            let b = p.stops[s].rgb;
            return a + (b - a) * f;
        }
    }
    return p.stops[n - 1].rgb;
}

fn rays_of(r: vec4<f32>) -> vec4<f32> {
    let g = p.gain;
    if (p.colourise == 0u) {
        return vec4<f32>(r.rgb * g, min(r.a * g, 1.0));
    }
    let v = max(r.r * g, 0.0);
    let a = clamp(r.a * g, 0.0, 1.0);
    let t = pow(min(v, 1.0), 1.0 / 2.2);
    let c = gradient(t);
    let k = max(v, 1.0) * a;
    return vec4<f32>(c * k, a);
}

fn combine(d: vec4<f32>, s: vec4<f32>) -> vec4<f32> {
    if (p.blend == 0u) {
        return s;
    }
    if (p.blend == 1u) {
        let k = 1.0 - min(s.a, 1.0);
        return vec4<f32>(s.rgb + d.rgb * k, min(s.a, 1.0) + d.a * k);
    }
    let sa = min(s.a, 1.0);
    var dc = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    if (d.a > 1e-6) {
        dc = vec4<f32>(d.rgb / d.a, 1.0);
    }
    var sc = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    if (sa > 1e-6) {
        sc = vec4<f32>(s.rgb / sa, 1.0);
    }
    let b = blend_pixel(p.blend - 2u, dc, sc);
    let rgb = s.rgb * (1.0 - d.a) + d.rgb * (1.0 - sa) + sa * d.a * b.rgb;
    return vec4<f32>(rgb, sa + d.a - sa * d.a);
}

@compute @workgroup_size(8, 8)
fn lr_combine(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(orig, xy, 0);
    var s = rays_of(textureLoad(src, xy, 0)) * p.rays_opacity;
    s.a = min(s.a, 1.0);
    let d = o * p.source_opacity;
    let out = combine(d, s);
    textureStore(dst, xy, o + (out - o) * p.mix_amt);
}

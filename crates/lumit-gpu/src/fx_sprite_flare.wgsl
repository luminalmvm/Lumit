// Sprite flare: every instance of the design drawn on every light as one
// instanced quad, added into a half-float target, then added to the layer.
//
// The twin of `lumit_core::fx::flare::raster`: `sf_vs` is its `place`,
// `sf_fs` its `shade`, step for step. The instance layout is
// `flare::instances::Instance::packed`.

struct Frame {
    w: f32,
    h: f32,
    centre: vec2<f32>,
    // Raster pixels; 0 is no fade.
    edge_fade: f32,
    lights: u32,
    _pad: vec2<f32>,
};

struct Inst {
    // anchor, at.x, at.y, flags
    a: vec4<f32>,
    // offset.xy, half.xy
    b: vec4<f32>,
    // angle, shape, grow, reach
    c: vec4<f32>,
    // colour, gain
    col: vec4<f32>,
    // colour at the edge, rainbow
    col_b: vec4<f32>,
    // response: mode, amount, range
    resp: vec4<f32>,
    s0: vec4<f32>,
    s1: vec4<f32>,
    s2: vec4<f32>,
};

struct Light {
    // x, y, gain
    pos: vec4<f32>,
    colour: vec4<f32>,
};

@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var<storage, read> insts: array<Inst>;
@group(0) @binding(2) var<storage, read> lights: array<Light>;

const PI: f32 = 3.14159265;
const TAU: f32 = 6.28318531;

struct Out {
    @builtin(position) pos: vec4<f32>,
    // uv across the quad, then its half size in pixels
    @location(0) uv_half: vec4<f32>,
    @location(1) colour: vec4<f32>,
    @location(2) colour_b: vec4<f32>,
    @location(3) @interpolate(flat) inst: u32,
    @location(4) @interpolate(flat) turn: f32,
};

fn ramp(a: f32, b: f32, x: f32) -> f32 {
    let t = clamp((x - a) / max(b - a, 1e-6), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

@vertex
fn sf_vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> Out {
    var o: Out;
    let n = max(frame.lights, 1u);
    let i = insts[ii / n];
    let l = lights[ii % n];
    let flags = u32(i.a.w);
    let axis = frame.centre - l.pos.xy;
    let axis_angle = atan2(axis.y, axis.x);
    let rho = length(axis) / max(0.5 * length(vec2<f32>(frame.w, frame.h)), 1.0);
    var at = i.a.yz;
    if (i.a.x < 0.5) {
        at = l.pos.xy;
    } else if (i.a.x < 1.5) {
        at = l.pos.xy + axis * i.a.y;
    }
    var gain = i.col.w * l.pos.z;
    var near = 1.0;
    if (i.resp.x > 1.5) {
        near = ramp(1.0 - i.resp.z, 1.0, rho);
    } else if (i.resp.x > 0.5) {
        near = 1.0 - ramp(0.0, i.resp.z, rho);
    }
    gain = gain * (1.0 - i.resp.y * (1.0 - near));
    if ((flags & 2u) == 0u && frame.edge_fade > 0.0) {
        let out_x = max(max(-l.pos.x, l.pos.x - frame.w), 0.0);
        let out_y = max(max(-l.pos.y, l.pos.y - frame.h), 0.0);
        gain = gain * (1.0 - ramp(0.0, frame.edge_fade, length(vec2<f32>(out_x, out_y))));
    }
    if (i.c.w > 0.0) {
        let d = length(at - l.pos.xy) / i.c.w;
        let lit = max(1.0 - d * d, 0.0);
        gain = gain * lit * lit;
    }
    if (gain <= 0.0) {
        o.pos = vec4<f32>(-2.0, -2.0, 0.0, 1.0);
        return o;
    }
    var angle = i.c.x;
    if ((flags & 1u) != 0u) {
        angle = angle + axis_angle;
    }
    let half = i.b.zw * (1.0 + i.c.z * rho);
    let s = sin(angle);
    let c = cos(angle);
    let centre = at + vec2<f32>(i.b.x * c - i.b.y * s, i.b.x * s + i.b.y * c);
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0),
        vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
    );
    let uv = corners[vi];
    let p = uv * half;
    let px = centre + vec2<f32>(p.x * c - p.y * s, p.x * s + p.y * c);
    o.pos = vec4<f32>(px.x / frame.w * 2.0 - 1.0, 1.0 - px.y / frame.h * 2.0, 0.0, 1.0);
    o.uv_half = vec4<f32>(uv, half);
    o.colour = vec4<f32>(i.col.xyz * l.colour.xyz, gain);
    o.colour_b = vec4<f32>(i.col_b.xyz * l.colour.xyz, i.col_b.w);
    o.inst = ii / n;
    o.turn = 0.0;
    if ((flags & 4u) != 0u) {
        o.turn = -axis_angle;
    }
    return o;
}

fn spectrum(x: f32) -> vec3<f32> {
    let t = clamp(x, 0.0, 1.0) * 4.0;
    return clamp(vec3<f32>(1.5) - abs(vec3<f32>(t) - vec3<f32>(3.2, 2.0, 0.8)), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn band(x: f32, width: f32, softness: f32) -> f32 {
    let wide = max(width, 1.0);
    let hard = clamp(wide * 0.5 - abs(x) + 0.5, 0.0, 1.0);
    let k = 2.0 * x / wide;
    let soft = exp(-(k * k));
    return (hard + (soft - hard) * softness) * (max(width, 0.0) / wide);
}

fn polygon(q: vec2<f32>, sides: f32, angle: f32, roundness: f32) -> f32 {
    let r = length(q);
    if (sides < 3.0) {
        return r;
    }
    let seg = TAU / sides;
    var a = atan2(q.y, q.x) - angle + seg * 0.5;
    a = a - seg * floor(a / seg) - seg * 0.5;
    let p = r * cos(a) / cos(seg * 0.5);
    return p + (r - p) * roundness;
}

// One channel of an iris: its brightness and how far out it is.
fn iris(q: vec2<f32>, s0: vec4<f32>, s1: vec4<f32>, angle: f32, feather: f32, rim_width: f32) -> vec2<f32> {
    let d = polygon(q, s0.y, angle, s0.w);
    let e = 1.0 - d;
    let cover = ramp(-feather * 0.5, feather * 0.5, e);
    let k = (e - rim_width) / rim_width;
    let rim = exp(-(k * k)) * cover;
    return vec2<f32>(s1.x * cover + s1.y * rim, d);
}

@fragment
fn sf_fs(f: Out) -> @location(0) vec4<f32> {
    let i = insts[f.inst];
    let uv = f.uv_half.xy;
    let half = f.uv_half.zw;
    let shape = u32(i.c.y);
    let len = length(uv);
    var v = vec3<f32>(0.0);
    var r = 0.0;
    if (shape == 0u) {
        let d = len * i.s0.x;
        let k = d / i.s0.w;
        v = vec3<f32>((exp(-pow(d, i.s0.y)) + i.s0.z * exp(-(k * k))) * (1.0 - ramp(0.9, 1.0, len)));
        r = min(d, 1.0);
    } else if (shape == 1u) {
        let two_way = i.s0.x > 0.5;
        var a = (uv.x + 1.0) * 0.5;
        var along = pow(max(1.0 - a, 0.0), i.s0.z);
        if (two_way) {
            a = abs(uv.x);
            along = exp(-pow(a * i.s0.y, i.s0.z)) * (1.0 - ramp(0.9, 1.0, a));
        }
        v = vec3<f32>(along * band(uv.y * half.y, i.s0.w * (1.0 - i.s1.y * a), i.s1.x));
        r = a;
    } else if (shape == 2u) {
        let x = (len - i.s0.x) * half.x;
        var b = band(x, i.s0.y * half.x, i.s0.z);
        if (i.s0.w < PI) {
            let aa = 1.0 / max(len * half.x, 1.0);
            let phi = abs(atan2(uv.y, uv.x));
            b = b * (1.0 - ramp(i.s0.w * (1.0 - i.s1.x) - aa, i.s0.w + aa, phi));
        }
        v = vec3<f32>(b);
        r = 0.5 + 0.5 * (len - i.s0.x) / max(i.s0.y, 1e-6);
    } else {
        let m = i.s0.x;
        let aa = m / max(min(half.x, half.y), 1e-3);
        let feather = max(i.s1.w, aa);
        let rim_width = max(i.s1.z, aa);
        let angle = i.s0.z + f.turn;
        let q = uv * m;
        let shift = i.s2.x;
        let g = iris(q, i.s0, i.s1, angle, feather, rim_width);
        var red = g.x;
        var blue = g.x;
        if (shift > 0.0) {
            red = iris(q - vec2<f32>(shift, 0.0), i.s0, i.s1, angle, feather, rim_width).x;
            blue = iris(q + vec2<f32>(shift, 0.0), i.s0, i.s1, angle, feather, rim_width).x;
        }
        v = vec3<f32>(red, g.x, blue);
        r = min(g.y, 1.0);
    }
    let base = mix(f.colour.xyz, f.colour_b.xyz, clamp(r, 0.0, 1.0));
    let tint = vec3<f32>(1.0) + (spectrum(r) - vec3<f32>(1.0)) * f.colour_b.w;
    return vec4<f32>(f.colour.w * base * tint * v, 0.0);
}

// The flare added to the layer: alpha is the layer's own.
@group(0) @binding(3) var src: texture_2d<f32>;
@group(0) @binding(4) var acc: texture_2d<f32>;
@group(0) @binding(5) var dst: texture_storage_2d<rgba16float, write>;

@compute @workgroup_size(8, 8)
fn sf_add(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = textureDimensions(src);
    if (gid.x >= size.x || gid.y >= size.y) {
        return;
    }
    let at = vec2<i32>(gid.xy);
    let base = textureLoad(src, at, 0);
    let light = textureLoad(acc, at, 0);
    textureStore(dst, at, vec4<f32>(base.rgb + light.rgb, base.a));
}

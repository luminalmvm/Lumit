// Sprite flare: the lights made ready, every instance of the design drawn on
// every light as one instanced quad, added into a half-float target, then
// added to the layer.
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
    // 1 when the Texture layer's picture is bound.
    has_layer: u32,
    _pad: f32,
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
    // x, y, gain, scale
    pos: vec4<f32>,
    colour: vec4<f32>,
};

@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var<storage, read> insts: array<Inst>;
@group(0) @binding(2) var<storage, read> lights: array<Light>;
// Every texture, 256 by 256 each and its smaller copies, one after another
// (`lumit_core::fx::flare::textures`).
@group(0) @binding(11) var<storage, read> texels: array<f32>;
// The Texture layer's picture, or any picture when there is none.
@group(0) @binding(12) var layer_tex: texture_2d<f32>;

const TEX_SIZE: i32 = 256;
// `textures::COUNT`, `RAINBOW`, `LAYER`, `LEVELS` and `STRIDE`.
const TEX_COUNT: u32 = 10u;
const TEX_RAINBOW: u32 = 8u;
const TEX_LAYER: u32 = 11u;
const TEX_LEVELS: u32 = 9u;
const TEX_STRIDE: i32 = 87381;

const PI: f32 = 3.14159265;
const TAU: f32 = 6.28318531;

struct Out {
    @builtin(position) pos: vec4<f32>,
    // The quad's middle and angle, and its half size in pixels: uv is found
    // from these at each pixel, as the CPU does, rather than interpolated
    // over corners the rasteriser has snapped, which a texture would show.
    @location(0) @interpolate(flat) place: vec4<f32>,
    @location(1) colour: vec4<f32>,
    @location(2) colour_b: vec4<f32>,
    @location(3) @interpolate(flat) inst: u32,
    @location(4) @interpolate(flat) turn: f32,
    @location(5) @interpolate(flat) light: vec2<f32>,
    @location(6) @interpolate(flat) extent: vec2<f32>,
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
    // What is spread across the frame stays its size whatever the light's.
    var scale = l.pos.w;
    if (i.a.x > 1.5) {
        scale = 1.0;
    }
    let extent = i.b.zw * (1.0 + i.c.z * rho) * scale;
    let s = sin(angle);
    let c = cos(angle);
    let off = i.b.xy * scale;
    let centre = at + vec2<f32>(off.x * c - off.y * s, off.x * s + off.y * c);
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0),
        vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
    );
    let uv = corners[vi];
    let p = uv * extent;
    let px = centre + vec2<f32>(p.x * c - p.y * s, p.x * s + p.y * c);
    o.pos = vec4<f32>(px.x / frame.w * 2.0 - 1.0, 1.0 - px.y / frame.h * 2.0, 0.0, 1.0);
    o.place = vec4<f32>(centre, angle, 0.0);
    o.extent = extent;
    o.colour = vec4<f32>(i.col.xyz * l.colour.xyz, gain);
    o.colour_b = vec4<f32>(i.col_b.xyz * l.colour.xyz, i.col_b.w);
    o.inst = ii / n;
    o.light = l.pos.xy;
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

fn polygon(q: vec2<f32>, sides: f32, angle: f32, roundness: f32, notches: f32) -> f32 {
    let r = length(q);
    if (sides < 3.0) {
        return r;
    }
    let seg = TAU / sides;
    var a = atan2(q.y, q.x) - angle + seg * 0.5;
    a = a - seg * floor(a / seg) - seg * 0.5;
    let corner = max(1.0 - (seg * 0.5 - abs(a)) / (seg * 0.1), 0.0);
    let p = r * cos(a) / cos(seg * 0.5) * (1.0 + 0.25 * notches * corner * corner);
    return p + (r - p) * roundness;
}

// Wrap `x` into 0..n.
fn wrap(x: i32, n: i32) -> i32 {
    return ((x % n) + n) % n;
}

// One copy of texture `k` at texel `p` of the full size, wrapping, bilinear
// between texel middles: `textures::sample_level`.
fn texture_level(k: u32, p: vec2<f32>, level: u32) -> f32 {
    let full = 4 * TEX_SIZE * TEX_SIZE;
    let base = i32(k - 1u) * TEX_STRIDE + (full - (full >> (2u * level))) / 3;
    let n = TEX_SIZE >> level;
    let q = p / f32(1u << level) - vec2<f32>(0.5);
    let q0 = floor(q);
    let f = q - q0;
    let x0 = i32(q0.x);
    let y0 = i32(q0.y);
    let xa = wrap(x0, n);
    let xb = wrap(x0 + 1, n);
    let ya = wrap(y0, n);
    let yb = wrap(y0 + 1, n);
    let a = texels[base + ya * n + xa];
    let b = texels[base + ya * n + xb];
    let c = texels[base + yb * n + xa];
    let d = texels[base + yb * n + xb];
    let top = a + (b - a) * f.x;
    let bottom = c + (d - c) * f.x;
    return top + (bottom - top) * f.y;
}

// Texture `k` (from 1) at texel `p`, from the copies that suit a pixel
// spanning `footprint` texels: `textures::sample`.
fn texture(k: u32, p: vec2<f32>, footprint: f32) -> f32 {
    if (k == 0u || k > TEX_COUNT) {
        return 0.0;
    }
    let lod = min(log2(max(footprint, 1.0)), f32(TEX_LEVELS - 1u));
    let level = floor(lod);
    let fine = texture_level(k, p, u32(level));
    let t = lod - level;
    if (t > 0.0) {
        return fine + (texture_level(k, p, u32(level) + 1u) - fine) * t;
    }
    return fine;
}

// The Texture layer at texel `p`, stretched so the square covers it once:
// `raster::Picture::at`.
fn layer_at(p: vec2<f32>) -> vec3<f32> {
    if (frame.has_layer == 0u) {
        return vec3<f32>(0.0);
    }
    let size = vec2<i32>(textureDimensions(layer_tex));
    let q = p / f32(TEX_SIZE) * vec2<f32>(size) - vec2<f32>(0.5);
    let q0 = floor(q);
    let f = q - q0;
    let xa = wrap(i32(q0.x), size.x);
    let xb = wrap(i32(q0.x) + 1, size.x);
    let ya = wrap(i32(q0.y), size.y);
    let yb = wrap(i32(q0.y) + 1, size.y);
    let a = max(textureLoad(layer_tex, vec2<i32>(xa, ya), 0).rgb, vec3<f32>(0.0));
    let b = max(textureLoad(layer_tex, vec2<i32>(xb, ya), 0).rgb, vec3<f32>(0.0));
    let c = max(textureLoad(layer_tex, vec2<i32>(xa, yb), 0).rgb, vec3<f32>(0.0));
    let d = max(textureLoad(layer_tex, vec2<i32>(xb, yb), 0).rgb, vec3<f32>(0.0));
    let top = a + (b - a) * f.x;
    let bottom = c + (d - c) * f.x;
    return top + (bottom - top) * f.y;
}

// `raster::texel`: a made texture, the Rainbow coloured, or the layer.
fn texel(k: u32, p: vec2<f32>, footprint: f32) -> vec3<f32> {
    if (k == TEX_LAYER) {
        return layer_at(p);
    }
    let t = texture(k, p, footprint);
    if (k == TEX_RAINBOW) {
        let n = f32(TEX_SIZE);
        let extent = n * 0.5;
        let w = p - n * floor(p / n);
        let d = length(w - vec2<f32>(extent)) / extent;
        return t * spectrum(clamp((d - 0.6) / 0.3, 0.0, 1.0));
    }
    return vec3<f32>(t);
}

// One channel of an iris: its brightness and how far out it is.
fn iris(q: vec2<f32>, s0: vec4<f32>, s1: vec4<f32>, angle: f32, feather: f32, rim_width: f32, notches: f32) -> vec2<f32> {
    let d = polygon(q, s0.y, angle, s0.w, notches);
    let e = 1.0 - d;
    let cover = ramp(-feather * 0.5, feather * 0.5, e);
    let k = (e - rim_width) / rim_width;
    let rim = exp(-(k * k)) * cover;
    return vec2<f32>(s1.x * cover + s1.y * rim, d);
}

// Texels a pixel spans on a sprite whose square fills its quad.
fn footprint(extent: vec2<f32>) -> f32 {
    return f32(TEX_SIZE) * 0.5 / max(min(extent.x, extent.y), 1e-3);
}

@fragment
fn sf_fs(f: Out) -> @location(0) vec4<f32> {
    let i = insts[f.inst];
    let extent = f.extent;
    let d = f.pos.xy - f.place.xy;
    let cs = cos(f.place.z);
    let sn = sin(f.place.z);
    let uv = vec2<f32>((d.x * cs + d.y * sn) / extent.x, (d.y * cs - d.x * sn) / extent.y);
    let shape = u32(i.c.y);
    let len = length(uv);
    var v = vec3<f32>(0.0);
    var r = 0.0;
    if (shape == 0u) {
        let d = len * i.s0.x;
        let k = d / i.s0.w;
        let g = (exp(-pow(d, i.s0.y)) + i.s0.z * exp(-(k * k))) * (1.0 - ramp(0.9, 1.0, len));
        let rim = i.s1.x * ramp(0.3, 0.9, d) * (1.0 - ramp(1.2, 2.0, d));
        v = g * (vec3<f32>(1.0) + (spectrum(d - 0.4) - vec3<f32>(1.0)) * rim);
        r = min(d, 1.0);
    } else if (shape == 1u) {
        let two_way = i.s0.x > 0.5;
        var a = (uv.x + 1.0) * 0.5;
        var along = pow(max(1.0 - a, 0.0), i.s0.z);
        var narrow = a;
        if (two_way) {
            a = abs(uv.x);
            along = exp(-pow(a * i.s0.y, i.s0.z)) * (1.0 - ramp(0.9, 1.0, a));
            narrow = min(a * i.s0.y * 0.5, 1.0);
        }
        let g = along * band(uv.y * extent.y, i.s0.w * (1.0 - i.s1.y * narrow), i.s1.x);
        v = g * (vec3<f32>(1.0) + (spectrum(a) - vec3<f32>(1.0)) * (i.s1.z * ramp(0.0, 0.6, a)));
        r = a;
    } else if (shape == 2u) {
        let spread = max(i.s1.y, 1.0);
        let x = (len - i.s0.x) * extent.x / spread;
        var b = band(x, i.s0.y * extent.x, i.s0.z) / sqrt(spread);
        if (i.s0.w < PI) {
            let aa = 1.0 / max(len * extent.x, 1.0);
            let phi = abs(atan2(uv.y, uv.x));
            b = b * (1.0 - ramp(i.s0.w * (1.0 - i.s1.x) - aa, i.s0.w + aa, phi));
        }
        v = vec3<f32>(b);
        r = 0.5 + 0.5 * (len - i.s0.x) / max(i.s0.y * spread, 1e-6);
    } else if (shape == 3u) {
        let m = i.s0.x;
        let aa = m / max(min(extent.x, extent.y), 1e-3);
        let feather = max(i.s1.w, aa);
        let rim_width = max(i.s1.z, aa);
        let angle = i.s0.z + f.turn;
        let q = uv * m;
        let shift = i.s2.x;
        let notches = i.s2.z;
        let g = iris(q, i.s0, i.s1, angle, feather, rim_width, notches);
        var red = g.x;
        var blue = g.x;
        if (shift > 0.0) {
            red = iris(q - vec2<f32>(shift, 0.0), i.s0, i.s1, angle, feather, rim_width, notches).x;
            blue = iris(q + vec2<f32>(shift, 0.0), i.s0, i.s1, angle, feather, rim_width, notches).x;
        }
        // A texture marks the glass rather than blacking it out.
        var marked = vec3<f32>(1.0);
        if (i.s2.y >= 0.5) {
            let k = u32(round(i.s2.y));
            let t = texel(k, (uv * 0.5 + vec2<f32>(0.5)) * f32(TEX_SIZE), footprint(extent));
            marked = vec3<f32>(0.3) + 0.7 * t;
        }
        v = vec3<f32>(red, g.x, blue) * marked;
        r = min(g.y, 1.0);
    } else {
        let k = u32(round(i.s0.x));
        if (i.s0.y > 0.5) {
            // Dirt: fixed to the frame, lit only near the light.
            let px = f.pos.xy;
            let rel = px - i.a.yz;
            let d = length(px - f.light) / max(i.s1.z, 1e-3);
            let lit = max(1.0 - d * d, 0.0);
            let sn = sin(i.s0.z);
            let cs = cos(i.s0.z);
            let t = vec2<f32>(rel.x * cs + rel.y * sn, rel.y * cs - rel.x * sn) * i.s0.w + i.s1.xy;
            v = texel(k, t, i.s0.w) * lit * lit;
            r = min(d, 1.0);
        } else {
            // A picture, fading out before the square's edge.
            let t = texel(k, (uv * 0.5 + vec2<f32>(0.5)) * f32(TEX_SIZE), footprint(extent));
            v = t * (1.0 - ramp(0.8, 1.0, len));
            r = min(len, 1.0);
        }
    }
    let base = mix(f.colour.xyz, f.colour_b.xyz, clamp(r, 0.0, 1.0));
    let tint = vec3<f32>(1.0) + (spectrum(r) - vec3<f32>(1.0)) * f.colour_b.w;
    return vec4<f32>(f.colour.w * base * tint * v, 0.0);
}

// The lights made ready, one thread each: the ones given, or Bright spots
// turned from what Lens flare's detection found, then each dimmed by how
// much of its disc Hide behind covers. `raster::spots` and
// `raster::covered` are the twins.
struct LightPass {
    count: u32,
    spots: u32,
    // 0 no hiding, 1 alpha, 2 luma.
    hide: u32,
    use_colour: u32,
    radius: f32,
    gain: f32,
    w: f32,
    h: f32,
    colour: vec4<f32>,
    // 1 when a depth is bound; a light behind what is nearer than this
    // many metres is hidden.
    depth: u32,
    light_m: f32,
    _p0: u32,
    _p1: u32,
};

// What detection writes: position as a fraction of the frame, colour, extent.
// Scalars only, so it packs eight floats a light as the detection's does.
struct Found {
    x: f32,
    y: f32,
    r: f32,
    g: f32,
    b: f32,
    ext_x: f32,
    ext_y: f32,
    _pad: f32,
};

@group(0) @binding(6) var<uniform> lp: LightPass;
@group(0) @binding(7) var<storage, read> given: array<Light>;
@group(0) @binding(8) var<storage, read> found: array<Found>;
@group(0) @binding(9) var hide_tex: texture_2d<f32>;
@group(0) @binding(10) var<storage, read_write> ready: array<Light>;
@group(0) @binding(13) var depth_tex: texture_2d<f32>;

const HIDE_TAPS: u32 = 64u;

@compute @workgroup_size(64)
fn sf_lights(@builtin(global_invocation_id) gid: vec3<u32>) {
    let k = gid.x;
    if (k >= lp.count) {
        return;
    }
    var l = given[k];
    if (lp.spots != 0u) {
        let s = found[k];
        let c = vec3<f32>(s.r, s.g, s.b);
        let m = max(max(c.r, c.g), c.b);
        var hue = vec3<f32>(1.0);
        if (lp.use_colour != 0u && m > 0.0) {
            hue = c / m;
        }
        l.pos = vec4<f32>(s.x * lp.w, s.y * lp.h, lp.gain * min(m, 1.0), 1.0);
        l.colour = vec4<f32>(hue * lp.colour.rgb, 0.0);
        if (m <= 0.0) {
            l.pos.z = 0.0;
        }
    }
    if (lp.hide != 0u && l.pos.z > 0.0) {
        let size = vec2<i32>(textureDimensions(hide_tex));
        var sum = 0.0;
        for (var t = 0u; t < HIDE_TAPS; t = t + 1u) {
            let r = max(lp.radius, 0.0) * sqrt((f32(t) + 0.5) / f32(HIDE_TAPS));
            let a = f32(t) * 2.399963;
            let p = floor(l.pos.xy + r * vec2<f32>(cos(a), sin(a)));
            let x = i32(p.x);
            let y = i32(p.y);
            if (p.x < 0.0 || p.y < 0.0 || x >= size.x || y >= size.y) {
                continue;
            }
            let c = textureLoad(hide_tex, vec2<i32>(x, y), 0);
            var v = c.a;
            if (lp.hide == 2u) {
                v = 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
            }
            sum = sum + clamp(v, 0.0, 1.0);
        }
        l.pos.z = l.pos.z * (1.0 - sum / f32(HIDE_TAPS));
    }
    // `raster::nearer`: the same taps, counting what is in front of it.
    if (lp.depth != 0u && l.pos.z > 0.0) {
        let size = vec2<i32>(textureDimensions(depth_tex));
        var sum = 0.0;
        for (var t = 0u; t < HIDE_TAPS; t = t + 1u) {
            let r = max(lp.radius, 0.0) * sqrt((f32(t) + 0.5) / f32(HIDE_TAPS));
            let a = f32(t) * 2.399963;
            let p = floor(l.pos.xy + r * vec2<f32>(cos(a), sin(a)));
            let x = i32(p.x);
            let y = i32(p.y);
            if (p.x < 0.0 || p.y < 0.0 || x >= size.x || y >= size.y) {
                continue;
            }
            if (max(textureLoad(depth_tex, vec2<i32>(x, y), 0).r, 0.01) < lp.light_m * 0.98) {
                sum = sum + 1.0;
            }
        }
        l.pos.z = l.pos.z * (1.0 - sum / f32(HIDE_TAPS));
    }
    ready[k] = l;
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

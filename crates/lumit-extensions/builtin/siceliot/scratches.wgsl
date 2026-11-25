// Scratches: thin curved strokes scattered at random, as Octane's
// Scratches. Each layer scatters one scratch per cell; every further layer's
// cells are Steps times smaller, so its scratches are finer and more. Curl
// bends them, Rotation sets their mean direction and Spread how far each
// strays from it.

struct Params {
    /// @counter(1, 8) @default(3) Layers
    layers: i32,
    /// @slider(4, 2000) @default(200) @unit(px) Length
    length: f32,
    /// @slider(0.2, 20) @default(1) @unit(px) Width
    width: f32,
    /// @bounded(0, 100) @default(70) Density
    density: f32,
    /// @slider(1, 4) @default(1.6) Steps
    steps: f32,
    /// @bounded(-100, 100) @default(20) Curl
    curl: f32,
    /// @dial @default(30) Rotation
    rotation: f32,
    /// @bounded(0, 100) @default(25) Spread
    spread: f32,
    /// @default(0.7, 0.7, 0.7, 1) Scratch colour
    colour: vec4<f32>,
    /// @default(0.02, 0.02, 0.022, 1) Background colour
    background: vec4<f32>,
    /// @seed Seed
    seed: u32,
    /// @choice("Inside layer", "Whole frame") @default("Inside layer") Paint
    paint: u32,
};

// A point in layer pixels turned by -angle about centre: the pattern turns.
fn sx_turn(px: vec2<f32>, centre: vec2<f32>, degrees: f32) -> vec2<f32> {
    let d = px - centre;
    let a = -radians(degrees);
    return vec2<f32>(d.x * cos(a) - d.y * sin(a), d.x * sin(a) + d.y * cos(a));
}

fn sx_paint(c: vec4<f32>, uv: vec2<f32>) -> vec4<f32> {
    if (p.paint == 0u) {
        return c * lumit_sample(uv).a;
    }
    return c;
}

// A hash of a lattice cell and a salt, 0 to 1, seeded by Seed and the layer.
fn sx_hash(c: vec2<i32>, salt: u32) -> f32 {
    var h = (bitcast<u32>(c.x) * 0x8da6b343u) ^ (bitcast<u32>(c.y) * 0xd8163841u) ^ (salt * 0xcb1ab31fu);
    h ^= (p.seed * 0x9e3779b9u) ^ lumit.seed;
    h ^= h >> 16u;
    h *= 0x7feb352du;
    h ^= h >> 15u;
    h *= 0x846ca68bu;
    h ^= h >> 16u;
    return f32(h >> 8u) / 16777216.0;
}

// Gradient noise, about -1 to 1, smooth to the second derivative.
fn sx_perlin(q: vec2<f32>, salt: u32) -> f32 {
    let i = vec2<i32>(floor(q));
    let f = q - floor(q);
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    var v = array<f32, 4>(0.0, 0.0, 0.0, 0.0);
    for (var k = 0; k < 4; k += 1) {
        let o = vec2<i32>(k & 1, k >> 1);
        let a = 6.2831853 * sx_hash(i + o, salt);
        v[k] = dot(vec2<f32>(cos(a), sin(a)), f - vec2<f32>(o));
    }
    return mix(mix(v[0], v[1], u.x), mix(v[2], v[3], u.x), u.y) * 1.4142135;
}

// Octaves of gradient noise, each twice as fine and `rough` times as strong,
// normalised to about -1 to 1. Each octave is shifted so no lattice lines meet.
fn sx_fbm(q: vec2<f32>, octaves: i32, rough: f32, salt: u32) -> f32 {
    var amp = 1.0;
    var freq = 1.0;
    var sum = 0.0;
    var norm = 0.0;
    for (var o = 0; o <= clamp(octaves, 0, 8); o += 1) {
        let shift = vec2<f32>(f32(o) * 0.7548777, f32(o) * 0.5698403);
        sum += sx_perlin(q * freq + shift, salt + u32(o) * 17u) * amp;
        norm += amp;
        amp *= rough;
        freq *= 2.0;
    }
    return sum / max(norm, 1e-6);
}


fn sc_one(q: vec2<f32>, c: vec2<i32>, cell: f32, len: f32, salt: u32) -> f32 {
    if (sx_hash(c, salt) > p.density / 100.0) {
        return 0.0;
    }
    let mid = (vec2<f32>(c) + vec2<f32>(sx_hash(c, salt + 1u), sx_hash(c, salt + 2u))) * cell;
    let a = radians(p.rotation) + (sx_hash(c, salt + 3u) - 0.5) * 3.1415927 * p.spread / 100.0;
    let dir = vec2<f32>(cos(a), sin(a));
    let d = q - mid;
    let x = dot(d, dir);
    let half = len * 0.5;
    if (abs(x) > half) {
        return 0.0;
    }
    // Bent into a parabola Curl of a length deep, each its own way.
    let bend = p.curl / 100.0 * (sx_hash(c, salt + 4u) * 2.0 - 1.0) * 0.5;
    let y = dot(d, vec2<f32>(-dir.y, dir.x)) - bend * (x * x / half - half * 0.5);
    let taper = 1.0 - pow(abs(x) / half, 4.0);
    let w = max(p.width * taper, 0.05) * 0.5;
    let px = 1.0 / max(lumit.comp_scale, 1e-3);
    let cover = clamp((w + px * 0.5 - abs(y)) / px, 0.0, 1.0);
    return cover * taper * (0.5 + 0.5 * sx_hash(c, salt + 5u));
}

fn shade(uv: vec2<f32>) -> vec4<f32> {
    let q = lumit_px(uv);
    var cover = 0.0;
    var len = max(p.length, 1.0);
    for (var l = 0; l < clamp(p.layers, 1, 8); l += 1) {
        // A scratch reaches at most half its length from its cell's point, so
        // cells as big as the scratch need only the neighbours.
        let cell = len;
        let base = vec2<i32>(floor(q / cell));
        let salt = 100u + u32(l) * 16u;
        for (var j = -1; j <= 1; j += 1) {
            for (var i = -1; i <= 1; i += 1) {
                cover = max(cover, sc_one(q, base + vec2<i32>(i, j), cell, len, salt));
            }
        }
        len /= max(p.steps, 1.0);
    }
    let c = mix(lumit_premult(p.background), lumit_premult(p.colour), cover);
    return sx_paint(c, uv);
}

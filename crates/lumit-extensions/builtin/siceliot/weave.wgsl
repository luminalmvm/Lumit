// Weave: a plain-woven cloth, as V-Ray's Cloth and Softimage's Fabric.
// Warp threads run up the layer and weft threads across it, Thread spacing
// apart; where they cross, each passes over one and under the next, so the
// two lock. Each thread is shaded round its width and dips where it goes
// under; Brightness and Width variation make every thread a little its own.

struct Params {
    /// @slider(2, 500) @default(40) @unit(px) Thread spacing
    spacing: f32,
    /// @bounded(5, 100) @default(80) Warp width
    warp_width: f32,
    /// @bounded(5, 100) @default(80) Weft width
    weft_width: f32,
    /// @default(0.55, 0.12, 0.08, 1) Warp colour
    warp_colour: vec4<f32>,
    /// @default(0.75, 0.6, 0.35, 1) Weft colour
    weft_colour: vec4<f32>,
    /// @default(0.02, 0.02, 0.02, 1) Gap colour
    gap_colour: vec4<f32>,
    /// @bounded(0, 100) @default(60) Thread shading
    shading: f32,
    /// @bounded(0, 100) @default(50) Wave
    wave: f32,
    /// @bounded(0, 100) @default(20) Brightness variation
    bright_var: f32,
    /// @bounded(0, 100) @default(10) Width variation
    width_var: f32,
    /// @dial @default(0) Rotation
    rotation: f32,
    /// @point @default(960, 540) Centre
    centre: vec2<f32>,
    /// @seed Seed
    seed: u32,
    /// @choice("Inside layer", "Whole frame") @default("Inside layer") Paint
    paint: u32,
};

// ---- The SICELIOT texture helpers: the same text in every SICELIOT texture ----

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
// ---- The SICELIOT texture noise: the same text in every SICELIOT texture ----

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


struct Thread {
    colour: vec4<f32>,
    cover: f32,
};

// across: -0.5 to 0.5 over the cell; along: the cell's position along the
// thread, in threads; index: which thread; shift: 0 for the warp, 1 for the
// weft, which is over where the warp is under.
fn wv_thread(across: f32, along: f32, index: i32, shift: i32, width: f32, colour: vec4<f32>, salt: u32) -> Thread {
    let w = width / 100.0 * (1.0 - p.width_var / 100.0 * sx_hash(vec2<i32>(index, 0), salt));
    let half = max(w * 0.5, 1e-3);
    let a = abs(across) / half;
    // A pixel's share of the thread's edge: one pixel wide at any size.
    let px = 1.0 / max(p.spacing * half * max(lumit.comp_scale, 1e-3), 1e-3);
    let cover = clamp((1.0 - a) / px, 0.0, 1.0);
    // Round across, and lower where it dips under: dip is 1 at the middle of
    // an over-crossing, 0 at the middle of an under-crossing.
    let bulge = sqrt(max(1.0 - a * a, 0.0));
    let ph = (along - 0.5 - f32((index + shift) & 1)) * 0.5;
    let dip = abs(ph - floor(ph) - 0.5) * 2.0;
    let lift = 1.0 - p.wave / 100.0 * 0.6 * (1.0 - dip);
    let lit = mix(1.0, 0.3 + 0.7 * bulge, p.shading / 100.0) * lift;
    let vary = 1.0 + p.bright_var / 100.0 * (sx_hash(vec2<i32>(index, 1), salt) - 0.5);
    let c = lumit_premult(colour);
    return Thread(vec4<f32>(c.rgb * lit * vary, c.a), cover);
}

fn shade(uv: vec2<f32>) -> vec4<f32> {
    let s = max(p.spacing, 1.0);
    let u = sx_turn(lumit_px(uv), p.centre, p.rotation) / s;
    let i = i32(floor(u.x));
    let j = i32(floor(u.y));
    let f = u - floor(u);
    let warp = wv_thread(f.x - 0.5, u.y, i, 0, p.warp_width, p.warp_colour, 5u);
    let weft = wv_thread(f.y - 0.5, u.x, j, 1, p.weft_width, p.weft_colour, 9u);
    var out = lumit_premult(p.gap_colour);
    if (((i + j) & 1) == 0) {
        out = mix(out, weft.colour, weft.cover);
        out = mix(out, warp.colour, warp.cover);
    } else {
        out = mix(out, warp.colour, warp.cover);
        out = mix(out, weft.colour, weft.cover);
    }
    return sx_paint(out, uv);
}

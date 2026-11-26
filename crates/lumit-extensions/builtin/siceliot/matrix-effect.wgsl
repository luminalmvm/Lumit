// Matrix effect: columns of glyphs raining down, as Octane's Matrix effect.
// Each column has its own speed and start; a bright head leads a trail that
// fades over Trail length glyphs, and the glyphs under it keep changing.
// The glyphs are made here, five by seven dots picked at random, so no font
// is needed. It runs with the comp's time; Speed 0 holds it still.

struct Params {
    /// @slider(4, 200) @default(20) @unit(px) Glyph size
    glyph_size: f32,
    /// @slider(0, 100) @default(12) Speed
    speed: f32,
    /// @slider(1, 100) @default(18) Trail length
    trail: f32,
    /// @bounded(0, 100) @default(70) Density
    density: f32,
    /// @slider(0, 30) @default(6) Glyph changes
    changes: f32,
    /// @default(0.2, 1.0, 0.35, 1) Text colour
    text: vec4<f32>,
    /// @default(0.85, 1.0, 0.9, 1) Head colour
    head: vec4<f32>,
    /// @default(0, 0, 0, 1) Background colour
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


fn mx_dot(g: vec2<i32>, x: i32, y: i32) -> bool {
    // Mirrored left to right about the middle column, as most glyphs are,
    // which reads as writing rather than static.
    let col = select(x, 4 - x, x > 2);
    return sx_hash(g * 7 + vec2<i32>(col, y), 9u) < 0.45;
}

fn shade(uv: vec2<f32>) -> vec4<f32> {
    let q = lumit_px(uv);
    let size = max(p.glyph_size, 1.0);
    // Glyph cells are 5 wide to 7 tall in dots, one dot of gap round.
    let cell = vec2<f32>(size * 0.75, size);
    let col = i32(floor(q.x / cell.x));
    let row = floor(q.y / cell.y);
    var c = lumit_premult(p.background);
    if (sx_hash(vec2<i32>(col, 0), 1u) > p.density / 100.0) {
        return sx_paint(c, uv);
    }
    let rows = lumit_size().y / max(lumit.comp_scale, 1e-6) / cell.y;
    let trail = max(p.trail, 1.0);
    let period = rows + trail;
    let pace = p.speed * (0.6 + 0.8 * sx_hash(vec2<i32>(col, 0), 2u));
    let start = sx_hash(vec2<i32>(col, 0), 3u) * period;
    let head = (lumit.time * pace + start) % period;
    let behind = head - row;
    if (behind < 0.0 || behind > trail) {
        return sx_paint(c, uv);
    }
    let fade = 1.0 - behind / trail;
    let tick = i32(floor(lumit.time * p.changes * (0.5 + sx_hash(vec2<i32>(col, i32(row)), 4u))));
    let g = vec2<i32>(col * 131 + tick, i32(row));
    let f = (q - vec2<f32>(f32(col), row) * cell) / cell * vec2<f32>(6.0, 8.0) - vec2<f32>(0.5);
    let dx = i32(floor(f.x));
    let dy = i32(floor(f.y));
    if (dx < 0 || dx > 4 || dy < 0 || dy > 6 || !mx_dot(g, dx, dy)) {
        return sx_paint(c, uv);
    }
    let ink = select(lumit_premult(p.text), lumit_premult(p.head), behind < 1.0);
    c = mix(c, ink, select(fade, 1.0, behind < 1.0));
    return sx_paint(c, uv);
}

// Woodgrain: a plank cut along a tree, as Octane's Woodgrain. The growth
// rings are circles round the trunk; the plank cuts them at Plank depth from
// the trunk's middle, so they show as long stripes that widen into arches
// where the trunk wobbles. The ring noise roughens them, the grain adds fine
// streaks along the plank.

struct Params {
    /// @default(0.50, 0.27, 0.11, 1) Light wood
    light: vec4<f32>,
    /// @default(0.16, 0.065, 0.02, 1) Dark wood
    dark: vec4<f32>,
    /// @slider(2, 400) @default(24) @unit(px) Ring spacing
    ring_spacing: f32,
    /// @bounded(0, 100) @default(40) Ring sharpness
    sharpness: f32,
    /// @bounded(0, 100) @default(80) Ring intensity
    ring_intensity: f32,
    /// @bounded(0, 100) @default(30) Ring unevenness
    unevenness: f32,
    /// @bounded(0, 100) @default(20) Ring noise
    ring_noise: f32,
    /// @slider(4, 1000) @default(60) @unit(px) Ring noise size
    ring_noise_size: f32,
    /// @bounded(0, 100) @default(35) Graininess
    graininess: f32,
    /// @slider(1, 100) @default(3) @unit(px) Grain size
    grain_size: f32,
    /// @slider(0, 2000) @default(160) @unit(px) Plank depth
    depth: f32,
    /// @slider(0, 500) @default(60) @unit(px) Trunk wobble
    trunk_wobble: f32,
    /// @slider(20, 4000) @default(900) @unit(px) Trunk wobble length
    trunk_wobble_length: f32,
    /// @bounded(0, 100) @default(25) Angular wobble
    angular_wobble: f32,
    /// @dial @default(0) Rotation
    rotation: f32,
    /// @point @default(960, 540) Trunk centre
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


fn shade(uv: vec2<f32>) -> vec4<f32> {
    // x runs along the trunk, y across it.
    let q = sx_turn(lumit_px(uv), p.centre, p.rotation);
    let along = q.x / max(p.trunk_wobble_length, 1.0);
    let sway = p.trunk_wobble * vec2<f32>(sx_perlin(vec2<f32>(along, 0.5), 1u), sx_perlin(vec2<f32>(along, 7.5), 2u));
    let cross = vec2<f32>(q.y + sway.x, p.depth + sway.y);
    var r = length(cross);
    let ang = atan2(cross.y, cross.x);
    r *= 1.0 + 0.15 * p.angular_wobble / 100.0 * sx_perlin(vec2<f32>(ang * 2.0, along * 3.0), 3u);
    let spacing = max(p.ring_spacing, 1.0);
    var rings = r / spacing;
    rings += p.unevenness / 100.0 * 2.0 * sx_perlin(vec2<f32>(rings * 0.15, 0.25), 4u);
    rings += p.ring_noise / 100.0 * sx_fbm(q / max(p.ring_noise_size, 1.0), 3, 0.5, 5u);
    let f = fract(rings);
    var t = pow(f, mix(1.0, 8.0, p.sharpness / 100.0)) * p.ring_intensity / 100.0;
    let g = sx_fbm(vec2<f32>(q.x / (max(p.grain_size, 0.5) * 40.0), q.y / max(p.grain_size, 0.5)), 2, 0.5, 6u);
    t = clamp(t + g * 0.5 * p.graininess / 100.0, 0.0, 1.0);
    let c = mix(lumit_premult(p.light), lumit_premult(p.dark), t);
    return sx_paint(c, uv);
}

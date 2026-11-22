// Flakes: the glittering flakes of metallic paint, as Octane's Flakes. One
// flake per cell at a random spot, each a round chip of its own size that
// catches the light at its own angle, so some flash and some stay dark.
// Blend fades the flakes back into the base colour.

struct Params {
    /// @default(0.008, 0.012, 0.06, 1) Base colour
    base: vec4<f32>,
    /// @default(0.7, 0.75, 1.0, 1) Flake colour
    flake: vec4<f32>,
    /// @slider(1, 100) @default(6) @unit(px) Flake size
    size: f32,
    /// @bounded(0, 100) @default(50) Flake size variance
    variance: f32,
    /// @bounded(0, 100) @default(80) Density
    density: f32,
    /// @bounded(0, 100) @default(60) Sparkle
    sparkle: f32,
    /// @bounded(0, 100) @default(0) Blend factor
    blend_factor: f32,
    /// @dial @default(0) Light angle
    light: f32,
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

// The finished colour, kept inside the layer when Paint asks for it.
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
    let q = lumit_px(uv);
    let cell = max(p.size, 1.0) * 1.5;
    let base = vec2<i32>(floor(q / cell));
    var cover = 0.0;
    var shine = 0.0;
    let px = 1.0 / max(lumit.comp_scale, 1e-3);
    for (var j = -1; j <= 1; j += 1) {
        for (var i = -1; i <= 1; i += 1) {
            let c = base + vec2<i32>(i, j);
            if (sx_hash(c, 1u) > p.density / 100.0) {
                continue;
            }
            let at = (vec2<f32>(c) + vec2<f32>(sx_hash(c, 2u), sx_hash(c, 3u))) * cell;
            let r = max(p.size, 1.0) * 0.5 * (1.0 + (sx_hash(c, 4u) - 0.5) * 2.0 * p.variance / 100.0 * 0.8);
            let k = clamp((r - distance(q, at)) / px + 0.5, 0.0, 1.0);
            if (k > cover) {
                cover = k;
                // Each flake's tilt is a direction; it shines as it faces the
                // light, and Sparkle narrows that to a flash.
                let tilt = 6.2831853 * sx_hash(c, 5u);
                let facing = 0.5 + 0.5 * cos(tilt - radians(p.light));
                shine = pow(facing, mix(1.0, 12.0, p.sparkle / 100.0));
            }
        }
    }
    let base_c = lumit_premult(p.base);
    let flake_c = lumit_premult(p.flake);
    let lit = mix(base_c, flake_c, shine);
    let c = mix(base_c, mix(lit, base_c, p.blend_factor / 100.0), cover);
    return sx_paint(c, uv);
}

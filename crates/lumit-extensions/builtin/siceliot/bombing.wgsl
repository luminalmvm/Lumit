// Bombing: copies of a picture scattered over the layer, as LuxCore's Bombing
// texture. The floor is cut into cells Spacing wide; each cell keeps one copy
// as often as Density says, moved inside its cell by Jitter, scaled down at
// random by up to Scale variation and, with Random rotation, turned at random.
// A copy is the square Source size wide round Source centre in the Bullet
// layer, drawn Bullet size wide. With no Bullet layer chosen a soft round
// dot stands in. Copies overlap; one cell's copy lies over another's in an
// order the seed sets.

struct Params {
    /// @slider(8, 2000) @default(160) @unit(px) Spacing
    spacing: f32,
    /// @slider(4, 2000) @default(120) @unit(px) Bullet size
    bullet_size: f32,
    /// @bounded(0, 100) @default(100) Density
    density: f32,
    /// @bounded(0, 100) @default(100) Jitter
    jitter: f32,
    /// @bounded(0, 100) @default(50) Scale variation
    scale_variation: f32,
    /// @toggle @default(1) Random rotation
    random_rotation: u32,
    /// @point @default(960, 540) Source centre
    source_centre: vec2<f32>,
    /// @slider(4, 4000) @default(400) @unit(px) Source size
    source_size: f32,
    /// @default(0.9, 0.3, 0.03, 1) Dot colour
    dot_colour: vec4<f32>,
    /// @choice("Over layer", "Bullets only") @default("Over layer") Composite
    composite: u32,
    /// @dial @default(0) Rotation
    rotation: f32,
    /// @point @default(960, 540) Centre
    centre: vec2<f32>,
    /// @seed Seed
    seed: u32,
};

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


// One copy's picture at a point in its own frame, -0.5 to 0.5 across.
fn bb_bullet(f: vec2<f32>) -> vec4<f32> {
    if (lumit.input2_on > 0.5) {
        let px = p.source_centre + f * p.source_size;
        let uv = px * max(lumit.comp_scale, 1e-6) / lumit_size();
        if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))) {
            return vec4<f32>(0.0);
        }
        return lumit_sample2(uv);
    }
    let r = length(f) * 2.0;
    let a = 1.0 - smoothstep(0.7, 1.0, r);
    return lumit_premult(vec4<f32>(p.dot_colour.rgb, p.dot_colour.a * a));
}

fn shade(uv: vec2<f32>) -> vec4<f32> {
    let px = lumit_px(uv);
    let d = px - p.centre;
    let a = -radians(p.rotation);
    let q = vec2<f32>(d.x * cos(a) - d.y * sin(a), d.x * sin(a) + d.y * cos(a));
    let s = max(p.spacing, 1.0);
    let base = vec2<i32>(floor(q / s));
    // How many cells away a copy can still reach: a copy is at most Bullet
    // size wide, turned it reaches its diagonal.
    let reach = i32(ceil(max(p.bullet_size, 1.0) * 0.7071068 / s + 1.0));
    let r = min(reach, 3);
    var acc = vec4<f32>(0.0);
    // Copies are laid in the seed's order: each pixel keeps them sorted by a
    // per-cell number, compositing the ones above over the ones below.
    var layers = array<vec4<f32>, 4>(vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0));
    var orders = array<f32, 4>(-1.0, -1.0, -1.0, -1.0);
    for (var dy = -r; dy <= r; dy += 1) {
        for (var dx = -r; dx <= r; dx += 1) {
            let c = base + vec2<i32>(dx, dy);
            if (sx_hash(c, p.seed * 7u + 1u) * 100.0 >= p.density) {
                continue;
            }
            let j = vec2<f32>(sx_hash(c, p.seed * 7u + 2u), sx_hash(c, p.seed * 7u + 3u)) - 0.5;
            let at = (vec2<f32>(c) + 0.5 + j * p.jitter / 100.0) * s;
            let scale = 1.0 - sx_hash(c, p.seed * 7u + 4u) * p.scale_variation / 100.0;
            let size = max(p.bullet_size * scale, 1e-3);
            var f = (q - at) / size;
            if (p.random_rotation != 0u) {
                let t = sx_hash(c, p.seed * 7u + 5u) * 6.2831853;
                f = vec2<f32>(f.x * cos(t) - f.y * sin(t), f.x * sin(t) + f.y * cos(t));
            }
            if (any(abs(f) > vec2<f32>(0.5))) {
                continue;
            }
            let col = bb_bullet(f);
            if (col.a <= 0.0) {
                continue;
            }
            var item = col;
            var ord = sx_hash(c, p.seed * 7u + 6u);
            for (var k = 0; k < 4; k += 1) {
                if (ord > orders[k]) {
                    let tc = layers[k];
                    let to = orders[k];
                    layers[k] = item;
                    orders[k] = ord;
                    item = tc;
                    ord = to;
                }
            }
        }
    }
    for (var k = 3; k >= 0; k -= 1) {
        if (orders[k] >= 0.0) {
            acc = layers[k] + acc * (1.0 - layers[k].a);
        }
    }
    if (p.composite == 1u) {
        return acc;
    }
    let under = lumit_sample(uv);
    return acc + under * (1.0 - acc.a);
}

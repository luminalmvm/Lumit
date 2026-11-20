// Wave pattern: bands or rings of a repeating wave, as Blender's Wave
// texture and Octane's Wave pattern. The profile shapes one wave (Sine,
// Saw, Triangle, Square); Distortion bends the waves with noise, which with
// a Sine profile and a few Detail octaves is marble. Phase offset slides the
// waves along and loops every 360 degrees.

struct Params {
    /// @choice("Bands", "Rings") @default("Bands") Type
    kind: u32,
    /// @choice("Sine", "Saw", "Triangle", "Square") @default("Sine") Profile
    profile: u32,
    /// @slider(2, 2000) @default(120) @unit(px) Wavelength
    wavelength: f32,
    /// @dial @default(0) @enabled_if(kind=0) Angle
    angle: f32,
    /// @point @default(960, 540) Centre
    centre: vec2<f32>,
    /// @dial @default(0) Phase offset
    phase: f32,
    /// @slider(0, 20) @default(0) Distortion
    distortion: f32,
    /// @counter(0, 8) @default(2) Detail
    detail: i32,
    /// @slider(4, 2000) @default(240) @unit(px) Detail scale
    detail_scale: f32,
    /// @bounded(0, 100) @default(50) Roughness
    roughness: f32,
    /// @seed Seed
    seed: u32,
    /// @choice("Inside layer", "Whole frame") @default("Inside layer") Paint
    paint: u32,
    /// @choice("Linear", "Smooth", "Constant") @default("Linear") Interpolation
    interpolation: u32,
    /// @choice("Linear", "Quadratic", "Easing") @default("Linear") Progression
    progression: u32,
    /// @counter(2, 5) @default(3) Stops
    stops: i32,
    /// @default(1, 1, 1, 1) Colour 1
    colour_1: vec4<f32>,
    /// @bounded(0, 100) @default(0) Position 1
    position_1: f32,
    /// @default(0.9, 0.55, 0.02, 1) Colour 2
    colour_2: vec4<f32>,
    /// @bounded(0, 100) @default(50) Position 2
    position_2: f32,
    /// @default(0.0, 0.08, 0.6, 1) Colour 3
    colour_3: vec4<f32>,
    /// @bounded(0, 100) @default(100) Position 3
    position_3: f32,
    /// @default(0.6, 0.0, 0.3, 1) Colour 4
    colour_4: vec4<f32>,
    /// @bounded(0, 100) @default(100) Position 4
    position_4: f32,
    /// @default(0, 0, 0, 1) Colour 5
    colour_5: vec4<f32>,
    /// @bounded(0, 100) @default(100) Position 5
    position_5: f32,
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


// ---- The SICELIOT ramp: the same text in every SICELIOT gradient ----

// The first Stops colours, sorted by Position, read at t (0 to 1), after
// Progression has bent t: Quadratic squares it, Easing eases it in and out.
// Colours are mixed premultiplied, so a clear stop fades without a fringe.
fn sc_ramp(t_in: f32) -> vec4<f32> {
    var t = t_in;
    if (p.progression != 0u) {
        t = clamp(t, 0.0, 1.0);
        if (p.progression == 1u) {
            t = t * t;
        } else {
            t = t * t * (3.0 - 2.0 * t);
        }
    }
    let n = clamp(p.stops, 2, 5);
    var pos = array<f32, 5>(p.position_1, p.position_2, p.position_3, p.position_4, p.position_5);
    var col = array<vec4<f32>, 5>(p.colour_1, p.colour_2, p.colour_3, p.colour_4, p.colour_5);
    for (var i = 0; i < 5; i += 1) {
        pos[i] = pos[i] / 100.0;
        col[i] = lumit_premult(col[i]);
    }
    for (var i = 1; i < n; i += 1) {
        var j = i;
        loop {
            if (j <= 0 || pos[j - 1] <= pos[j]) {
                break;
            }
            let tp = pos[j];
            pos[j] = pos[j - 1];
            pos[j - 1] = tp;
            let tc = col[j];
            col[j] = col[j - 1];
            col[j - 1] = tc;
            j -= 1;
        }
    }
    if (t <= pos[0]) {
        return col[0];
    }
    for (var i = 1; i < n; i += 1) {
        if (t <= pos[i]) {
            if (p.interpolation == 2u) {
                return col[i - 1];
            }
            var f = (t - pos[i - 1]) / max(pos[i] - pos[i - 1], 1e-6);
            if (p.interpolation == 1u) {
                f = f * f * (3.0 - 2.0 * f);
            }
            return mix(col[i - 1], col[i], f);
        }
    }
    return col[n - 1];
}

fn shade(uv: vec2<f32>) -> vec4<f32> {
    let q = sx_turn(lumit_px(uv), p.centre, select(0.0, p.angle, p.kind == 0u));
    let len = max(p.wavelength, 1.0);
    var n = q.x / len;
    if (p.kind == 1u) {
        n = length(q) / len;
    }
    if (p.distortion > 0.0) {
        n += p.distortion * sx_fbm(q / max(p.detail_scale, 1.0), p.detail, p.roughness / 100.0, 5u);
    }
    n += p.phase / 360.0;
    let f = fract(n);
    // One pixel of the wave, in waves: the hard edges of Saw and Square are
    // smoothed over it, so they never shimmer.
    let aa = 1.0 / (len * max(lumit.comp_scale, 1e-3));
    var t = 0.5 - 0.5 * cos(6.2831853 * n);
    switch p.profile {
        case 1u: {
            // The ramp up, and the drop back smoothed over a pixel.
            t = f * clamp((1.0 - f) / aa, 0.0, 1.0);
        }
        case 2u: {
            t = 1.0 - abs(f * 2.0 - 1.0);
        }
        case 3u: {
            let up = clamp((f - 0.0) / aa, 0.0, 1.0);
            let down = clamp((f - 0.5) / aa, 0.0, 1.0);
            t = up - down;
        }
        default: {}
    }
    return sx_paint(sc_ramp(t), uv);
}

// Dithering: fewer colours, with the steps between them broken into a fine
// pattern, as Cavalry's Dithering Filter. Ordered uses an 8 x 8 Bayer
// pattern; Noise uses a fixed random one.

struct Params {
    /// @choice("Ordered", "Noise") @default("Ordered") Method
    method: u32,
    /// @counter(2, 16) @default(2) Levels
    levels: i32,
    /// @slider(1, 32) @default(2) @unit(px) Pixel size
    pixel_size: f32,
    /// @choice("Colour", "Monochrome") @default("Colour") Colours
    colours: u32,
    /// @default(0.02, 0.02, 0.06, 1) Dark
    dark: vec4<f32>,
    /// @default(0.95, 0.9, 0.75, 1) Light
    light: vec4<f32>,
    /// @seed Seed
    seed: u32,
};

// The 8 x 8 Bayer threshold for a cell, 0 to 1.
fn di_bayer(c: vec2<u32>) -> f32 {
    var v = 0u;
    for (var b = 0u; b < 3u; b += 1u) {
        let x = (c.x >> b) & 1u;
        let y = (c.y >> b) & 1u;
        v = v * 4u + 2u * (x ^ y) + y;
    }
    return (f32(v) + 0.5) / 64.0;
}

fn di_noise(c: vec2<u32>) -> f32 {
    var h = (c.x * 0x8da6b343u) ^ (c.y * 0xd8163841u) ^ (p.seed * 0x9e3779b9u) ^ lumit.seed;
    h ^= h >> 16u;
    h *= 0x7feb352du;
    h ^= h >> 15u;
    h *= 0x846ca68bu;
    h ^= h >> 16u;
    return f32(h >> 8u) / 16777216.0;
}

fn shade(uv: vec2<f32>) -> vec4<f32> {
    let size = max(p.pixel_size, 1.0);
    let cell = floor(lumit_px(uv) / size);
    let mid_uv = (cell + 0.5) * size / max(lumit_size() / max(lumit.comp_scale, 1e-6), vec2<f32>(1.0));
    let here = lumit_sample(clamp(mid_uv, vec2<f32>(0.0), vec2<f32>(1.0)));
    let straight = lumit_unpremult(here);
    let ci = vec2<u32>(vec2<i32>(cell) & vec2<i32>(0xffff));
    let threshold = select(di_bayer(ci & vec2<u32>(7u)), di_noise(ci), p.method == 1u);
    let steps = f32(max(p.levels, 2) - 1);
    let alpha = lumit_sample(uv).a;
    if (p.colours == 1u) {
        let v = clamp(dot(straight.rgb, vec3<f32>(0.2126, 0.7152, 0.0722)), 0.0, 1.0);
        let q = clamp(floor(v * steps + threshold) / steps, 0.0, 1.0);
        return mix(lumit_premult(p.dark), lumit_premult(p.light), q) * alpha;
    }
    let rgb = clamp(floor(clamp(straight.rgb, vec3<f32>(0.0), vec3<f32>(1.0)) * steps + threshold) / steps, vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(rgb * alpha, alpha);
}

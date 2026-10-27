// Scrape: drags the pixels of a line through Centre out along Angle, fading
// as they go, as Cavalry's Scrape Filter. Roughness gives each row of pixels
// its own length, like paint pulled by a rough edge.

struct Params {
    /// @point @default(960, 540) Centre
    centre: vec2<f32>,
    /// @dial @default(0) Angle
    angle: f32,
    /// @slider(0, 4000) @default(600) @unit(px) Length
    length: f32,
    /// @bounded(0, 100) @default(30) Roughness
    roughness: f32,
    /// @bounded(0, 100) @default(50) Fade
    fade: f32,
    /// @seed Seed
    seed: u32,
};

fn sr_rand(row: i32) -> f32 {
    var h = (bitcast<u32>(row) * 0x8da6b343u) ^ (p.seed * 0x9e3779b9u) ^ lumit.seed;
    h ^= h >> 16u;
    h *= 0x7feb352du;
    h ^= h >> 15u;
    h *= 0x846ca68bu;
    h ^= h >> 16u;
    return f32(h >> 8u) / 16777216.0;
}

fn shade(uv: vec2<f32>) -> vec4<f32> {
    let here = lumit_sample(uv);
    let a = radians(p.angle);
    let along = vec2<f32>(cos(a), sin(a));
    let across = vec2<f32>(-along.y, along.x);
    let q = lumit_px(uv) - p.centre;
    let n = dot(q, along);
    let t = dot(q, across);
    let reach = p.length * (1.0 - p.roughness / 100.0 * sr_rand(i32(floor(t))));
    if (n <= 0.0 || n > reach) {
        return here;
    }
    let line_px = p.centre + across * t;
    let at = clamp(line_px * lumit.comp_scale / lumit_size(), vec2<f32>(0.0), vec2<f32>(1.0));
    let f = 1.0 - p.fade / 100.0 * n / max(reach, 1e-3);
    return mix(here, lumit_sample(at), f);
}

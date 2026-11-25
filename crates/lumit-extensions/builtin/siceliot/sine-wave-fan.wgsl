// Sine wave fan: rays fanning out from Centre, each bent by a sine wave
// that travels outward, as Octane's Sine wave fan. Stripes is how many waves
// fit along Wave length; Movement slides them out and loops every 360
// degrees. Intensity sharpens the rays from soft bands to thin lines.

struct Params {
    /// @counter(1, 128) @default(16) Rays
    rays: i32,
    /// @dial @default(0) Rotation
    rotation: f32,
    /// @bounded(0, 100) @default(30) Wave amount
    amount: f32,
    /// @slider(4, 4000) @default(300) @unit(px) Wave length
    wave_length: f32,
    /// @slider(0.25, 16) @default(1) Stripes
    stripes: f32,
    /// @dial @default(0) Movement
    movement: f32,
    /// @bounded(0, 100) @default(50) Intensity
    intensity: f32,
    /// @default(1, 0.85, 0.3, 1) Colour
    colour: vec4<f32>,
    /// @default(0.006, 0.003, 0.015, 1) Background colour
    background: vec4<f32>,
    /// @point @default(960, 540) Centre
    centre: vec2<f32>,
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

fn shade(uv: vec2<f32>) -> vec4<f32> {
    let d = lumit_px(uv) - p.centre;
    let r = length(d);
    let ang = atan2(d.y, d.x) - radians(p.rotation);
    let n = f32(clamp(p.rays, 1, 128));
    let wave = sin(6.2831853 * (r / max(p.wave_length, 1.0) * p.stripes) - radians(p.movement));
    // Bent by up to half a ray's width either way.
    let v = 0.5 + 0.5 * cos(n * ang + 3.1415927 * p.amount / 100.0 * wave);
    let t = pow(v, mix(1.0, 24.0, p.intensity / 100.0));
    let c = mix(lumit_premult(p.background), lumit_premult(p.colour), t);
    return sx_paint(c, uv);
}

// Light sweep: a soft band of light across the layer, as Cavalry's Light
// Sweep Filter. Keyframe Centre to sweep it over. The light only lands where
// the layer draws.

struct Params {
    /// @point @default(960, 540) Centre
    centre: vec2<f32>,
    /// @dial @default(30) Angle
    angle: f32,
    /// @slider(0, 2000) @default(160) @unit(px) Width
    width: f32,
    /// @bounded(0, 100) @default(60) Softness
    softness: f32,
    /// @default(1, 1, 1, 1) Colour
    colour: vec4<f32>,
    /// @slider(0, 400) @default(100) Intensity
    intensity: f32,
    /// @choice("Add", "Screen") @default("Add") Mode
    mode: u32,
};

fn shade(uv: vec2<f32>) -> vec4<f32> {
    let here = lumit_sample(uv);
    let a = radians(p.angle);
    // Distance from the band's middle line, measured along Angle.
    let d = abs(dot(lumit_px(uv) - p.centre, vec2<f32>(cos(a), sin(a))));
    let half = max(p.width * 0.5, 1e-3);
    let inner = half * (1.0 - p.softness / 100.0);
    let band = 1.0 - smoothstep(inner, half + 1e-3, d);
    let light = p.colour.rgb * p.colour.a * band * p.intensity / 100.0 * here.a;
    var rgb = here.rgb + light;
    if (p.mode == 1u) {
        rgb = here.rgb + light - here.rgb * clamp(light, vec3<f32>(0.0), vec3<f32>(1.0));
    }
    return vec4<f32>(rgb, here.a);
}

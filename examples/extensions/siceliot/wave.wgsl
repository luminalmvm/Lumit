// Wave: a wave runs through the picture and pushes it sideways, as Cavalry's
// Wave behaviour does to a path. Speed moves it on its own; Phase is the
// place to keyframe for an exact loop.

struct Params {
    /// @choice("Sine", "Triangle", "Square", "Sawtooth") @default("Sine") Shape
    shape: u32,
    /// @slider(0, 500) @default(20) @unit(px) Amplitude
    amplitude: f32,
    /// @slider(2, 4000) @default(200) @unit(px) Wavelength
    wavelength: f32,
    /// @dial @default(0) Angle
    angle: f32,
    /// @dial @default(0) Phase
    phase: f32,
    /// @slider(-10, 10) @default(1) Speed
    speed: f32,
};

fn wv_wave(x: f32) -> f32 {
    switch p.shape {
        case 1u: { return 4.0 * abs(fract(x - 0.25) - 0.5) - 1.0; }
        case 2u: { return select(-1.0, 1.0, fract(x) < 0.5); }
        case 3u: { return 2.0 * fract(x) - 1.0; }
        default: { return sin(6.2831853 * x); }
    }
}

fn shade(uv: vec2<f32>) -> vec4<f32> {
    let a = radians(p.angle);
    let along = vec2<f32>(cos(a), sin(a));
    let across = vec2<f32>(-along.y, along.x);
    let px = lumit_px(uv);
    let x = dot(px, along) / max(p.wavelength, 1.0) + p.phase / 360.0 + p.speed * lumit.time;
    let src = px - across * p.amplitude * wv_wave(x);
    let at = src * lumit.comp_scale / lumit_size();
    if (any(at < vec2<f32>(0.0)) || any(at > vec2<f32>(1.0))) {
        return vec4<f32>(0.0);
    }
    return lumit_sample(at);
}

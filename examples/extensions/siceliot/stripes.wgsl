// Stripes: reads a band of pixels through Centre and repeats it along Angle,
// as Cavalry's Stripes Filter. At a Width of 1 px every pixel of the line is
// pulled out into a streak.

struct Params {
    /// @point @default(960, 540) Centre
    centre: vec2<f32>,
    /// @dial @default(0) Angle
    angle: f32,
    /// @slider(1, 1000) @default(1) @unit(px) Width
    width: f32,
    /// @choice("Both sides", "Front", "Back") @default("Both sides") Side
    side: u32,
};

fn shade(uv: vec2<f32>) -> vec4<f32> {
    let a = radians(p.angle);
    let along = vec2<f32>(cos(a), sin(a));
    let across = vec2<f32>(-along.y, along.x);
    let q = lumit_px(uv) - p.centre;
    let n = dot(q, along);
    if ((p.side == 1u && n < 0.0) || (p.side == 2u && n > 0.0)) {
        return lumit_sample(uv);
    }
    let w = max(p.width, 1.0);
    let folded = n - w * floor(n / w + 0.5);
    let from_px = p.centre + across * dot(q, across) + along * folded;
    let at = clamp(from_px * lumit.comp_scale / lumit_size(), vec2<f32>(0.0), vec2<f32>(1.0));
    return lumit_sample(at);
}

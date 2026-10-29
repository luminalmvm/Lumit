// Inner shadow: a soft shadow cast inside the layer's edges, as Cavalry's
// Inner Shadow Filter. The layer's outside is shifted by Distance towards
// Angle, blurred by Size in two half-size passes, and laid over the layer
// where it draws. Past the layer's own edges counts as outside.

struct Params {
    /// @default(0, 0, 0, 0.75) Colour
    colour: vec4<f32>,
    /// @dial @default(45) Angle
    angle: f32,
    /// @slider(0, 200) @default(12) @unit(px) Distance
    distance: f32,
    /// @slider(0, 200) @default(16) @unit(px) Size
    size: f32,
    /// @bounded(0, 100) @default(0) Choke
    choke: f32,
};

const IS_TAPS: i32 = 12;

// px at comp size as a uv step.
fn is_uv(px: vec2<f32>) -> vec2<f32> {
    return px * lumit.comp_scale / lumit_size();
}

fn is_outside(uv: vec2<f32>) -> f32 {
    if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))) {
        return 1.0;
    }
    return 1.0 - lumit_sample(uv).a;
}

fn is_weight(i: i32) -> f32 {
    let x = f32(i) / f32(IS_TAPS);
    return exp(-4.0 * x * x);
}

// Pass one, half size: the outside, moved and blurred across.
fn spread_x(uv: vec2<f32>) -> vec4<f32> {
    let dir = vec2<f32>(cos(radians(p.angle)), sin(radians(p.angle)));
    let start = uv - is_uv(dir * p.distance);
    let step = is_uv(vec2<f32>(p.size / f32(IS_TAPS), 0.0));
    var sum = 0.0;
    var total = 0.0;
    for (var i = -IS_TAPS; i <= IS_TAPS; i += 1) {
        let w = is_weight(i);
        sum += is_outside(start + step * f32(i)) * w;
        total += w;
    }
    let v = sum / total;
    return vec4<f32>(v, v, v, 1.0);
}

// Pass two, half size: the same blur down.
fn spread_y(uv: vec2<f32>) -> vec4<f32> {
    let step = is_uv(vec2<f32>(0.0, p.size / f32(IS_TAPS)));
    var sum = 0.0;
    var total = 0.0;
    for (var i = -IS_TAPS; i <= IS_TAPS; i += 1) {
        let w = is_weight(i);
        sum += lumit_read(pass_spread_x, uv + step * f32(i)).r * w;
        total += w;
    }
    let v = sum / total;
    return vec4<f32>(v, v, v, 1.0);
}

// Pass three: the shadow over the layer, inside it only.
fn shade(uv: vec2<f32>) -> vec4<f32> {
    let here = lumit_sample(uv);
    var s = lumit_read(pass_spread_y, uv).r;
    s = clamp(s / (1.0 - 0.95 * p.choke / 100.0), 0.0, 1.0);
    let a = s * p.colour.a;
    return vec4<f32>(here.rgb * (1.0 - a) + p.colour.rgb * a * here.a, here.a);
}

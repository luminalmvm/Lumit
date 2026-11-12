// DisplaceX: a map moves, turns and scales each pixel of the layer, where
// Displacement map only moves it. The map's value at a pixel sets how much of
// the transform that pixel takes: in Standard, grey takes none, white all of
// it and black all of it backwards; in Legacy, black takes none and white all.
// The map is the layer itself, another layer, or a ramp. Softness blurs the
// map before it is read. Iterations average the transform from the full
// amount back toward none, across Shutter duration, for a smooth or
// motion-blurred result. Chromatic aberration gives each colour channel its
// own share of the transform.

struct Params {
    /// @choice("Layer itself", "Map layer", "Gradient ramp") @default("Layer itself") Map
    map: u32,
    /// @choice("Luminance", "Red", "Green", "Blue", "Alpha", "Hue", "Saturation", "Lightness") @default("Luminance") @enabled_if(map != 2) Channel
    channel: u32,
    /// @choice("Radial", "Linear") @default("Radial") @enabled_if(map = 2) Ramp shape
    ramp_shape: u32,
    /// @bounded(-50, 150) @default(50) @enabled_if(map = 2) Ramp origin X (%)
    ramp_x: f32,
    /// @bounded(-50, 150) @default(50) @enabled_if(map = 2) Ramp origin Y (%)
    ramp_y: f32,
    /// @slider(1, 4000) @default(500) @unit(px) @enabled_if(map = 2) Ramp scale
    ramp_scale: f32,
    /// @dial @default(0) @enabled_if(map = 2, ramp_shape = 1) Ramp angle
    ramp_angle: f32,
    /// @choice("Standard", "Legacy") @default("Standard") Map interpretation
    mode: u32,
    /// @toggle @default(0) Invert map
    invert: u32,
    /// @bounded(0, 100) @default(0) @enabled_if(map != 0) Map mix input
    mix_input: f32,
    /// @slider(0, 200) @default(0) @unit(px) Map softness
    softness: f32,
    /// @bounded(0.1, 10) @default(1) Map gamma
    gamma: f32,
    /// @slider(-200, 200) @default(0) Displacement offset (%)
    offset: f32,
    /// @choice("Linear", "Ease in", "Ease out", "Ease in and out") @default("Linear") Displacement easing
    easing: u32,
    /// @slider(-2000, 2000) @default(0) @unit(px) Translate X
    translate_x: f32,
    /// @slider(-2000, 2000) @default(0) @unit(px) Translate Y
    translate_y: f32,
    /// @dial @default(0) Rotate
    rotate: f32,
    /// @toggle @default(1) Uniform scale
    uniform: u32,
    /// @slider(-400, 400) @default(100) @enabled_if(uniform = 1) Scale (%)
    scale: f32,
    /// @slider(-400, 400) @default(100) @enabled_if(uniform = 0) Scale X (%)
    scale_x: f32,
    /// @slider(-400, 400) @default(100) @enabled_if(uniform = 0) Scale Y (%)
    scale_y: f32,
    /// @toggle @default(0) @enabled_if(map = 2) Link anchor to origin
    link_anchor: u32,
    /// @bounded(-50, 150) @default(50) Anchor X (%)
    anchor_x: f32,
    /// @bounded(-50, 150) @default(50) Anchor Y (%)
    anchor_y: f32,
    /// @toggle @default(0) Clamp negative scale
    clamp_negative: u32,
    /// @slider(-10, 10) @default(1) Translate multiplier
    mult_translate: f32,
    /// @slider(-10, 10) @default(1) Rotate multiplier
    mult_rotate: f32,
    /// @slider(-10, 10) @default(1) Scale multiplier
    mult_scale: f32,
    /// @counter(1, 32) @default(1) Iterations
    iterations: f32,
    /// @bounded(0, 100) @default(50) Shutter duration (%)
    shutter: f32,
    /// @slider(-5, 5) @default(1) Red
    red: f32,
    /// @slider(-5, 5) @default(1) Green
    green: f32,
    /// @slider(-5, 5) @default(1) Blue
    blue: f32,
    /// @choice("Transparent", "Clamp", "Repeat", "Mirror") @default("Transparent") Edge behaviour
    edge: u32,
    /// @slider(0, 2000) @default(0) @unit(px) Left
    left: f32,
    /// @slider(0, 2000) @default(0) @unit(px) Top
    top: f32,
    /// @slider(0, 2000) @default(0) @unit(px) Right
    right: f32,
    /// @slider(0, 2000) @default(0) @unit(px) Bottom
    bottom: f32,
    /// @choice("Off", "2x", "4x") @default("Off") Anti-aliasing
    antialias: u32,
    /// @choice("Result", "Map") @default("Result") View
    view: u32,
};

// The layer's size in comp pixels, which every distance here is measured in.
fn dx_frame() -> vec2<f32> {
    return lumit_size() / max(lumit.comp_scale, 1e-6);
}

// HSL of a straight colour.
fn dx_hsl(c: vec3<f32>) -> vec3<f32> {
    let hi = max(c.r, max(c.g, c.b));
    let lo = min(c.r, min(c.g, c.b));
    let l = (hi + lo) * 0.5;
    let d = hi - lo;
    if (d <= 1e-6) {
        return vec3<f32>(0.0, 0.0, l);
    }
    let s = d / max(1.0 - abs(2.0 * l - 1.0), 1e-6);
    var h = 0.0;
    if (hi == c.r) {
        h = (c.g - c.b) / d;
        if (h < 0.0) {
            h += 6.0;
        }
    } else if (hi == c.g) {
        h = (c.b - c.r) / d + 2.0;
    } else {
        h = (c.r - c.g) / d + 4.0;
    }
    return vec3<f32>(h / 6.0, clamp(s, 0.0, 1.0), l);
}

// One picture's value on the chosen channel.
fn dx_channel(premult: vec4<f32>) -> f32 {
    let c = lumit_unpremult(premult);
    switch p.channel {
        case 1u: { return c.r; }
        case 2u: { return c.g; }
        case 3u: { return c.b; }
        case 4u: { return premult.a; }
        case 5u: { return dx_hsl(c.rgb).x; }
        case 6u: { return dx_hsl(c.rgb).y; }
        case 7u: { return dx_hsl(c.rgb).z; }
        default: {
            // Luminance, dark where the layer is clear.
            return dot(premult.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
        }
    }
}

// The ramp's origin, in comp pixels.
fn dx_origin() -> vec2<f32> {
    return vec2<f32>(p.ramp_x, p.ramp_y) * 0.01 * dx_frame();
}

// The map's raw value at uv, before softness and the adjustments.
fn dx_raw(uv: vec2<f32>) -> f32 {
    var v = 0.0;
    if (p.map == 2u) {
        let d = uv * dx_frame() - dx_origin();
        let scale = max(p.ramp_scale, 1e-3);
        if (p.ramp_shape == 1u) {
            let a = radians(p.ramp_angle);
            v = dot(d, vec2<f32>(cos(a), sin(a))) / scale;
        } else {
            v = length(d) / scale;
        }
        v = clamp(v, 0.0, 1.0);
    } else {
        var m = lumit_sample(uv);
        if (p.map == 1u && lumit.input2_on > 0.5) {
            m = lumit_sample2(uv);
        }
        v = dx_channel(m);
        if (p.map != 0u) {
            v = mix(v, dx_channel(lumit_sample(uv)), clamp(p.mix_input * 0.01, 0.0, 1.0));
        }
    }
    if (p.invert != 0u) {
        v = 1.0 - v;
    }
    return clamp(v, 0.0, 1.0);
}

// Map softness as a Gaussian, half of it across and half down, in pass
// pixels: the passes are drawn at half size.
const DX_TAPS: i32 = 24;

fn dx_sigma() -> f32 {
    return max(p.softness * lumit.comp_scale * 0.5, 1e-3) / 2.0;
}

// Pass one: the map blurred across.
fn map_h(uv: vec2<f32>) -> vec4<f32> {
    let size = lumit_size() * 0.5;
    let sigma = dx_sigma();
    let reach = sigma * 3.0;
    var sum = 0.0;
    var weight = 0.0;
    for (var i = -DX_TAPS; i <= DX_TAPS; i += 1) {
        let off = f32(i) * reach / f32(DX_TAPS);
        let w = exp(-0.5 * off * off / (sigma * sigma));
        sum += w * dx_raw(uv + vec2<f32>(off / size.x, 0.0));
        weight += w;
    }
    return vec4<f32>(sum / max(weight, 1e-6), 0.0, 0.0, 1.0);
}

// Pass two: down, reading pass one.
fn map_v(uv: vec2<f32>) -> vec4<f32> {
    let size = lumit_size() * 0.5;
    let sigma = dx_sigma();
    let reach = sigma * 3.0;
    var sum = 0.0;
    var weight = 0.0;
    for (var i = -DX_TAPS; i <= DX_TAPS; i += 1) {
        let off = f32(i) * reach / f32(DX_TAPS);
        let w = exp(-0.5 * off * off / (sigma * sigma));
        sum += w * lumit_read(pass_map_h, uv + vec2<f32>(0.0, off / size.y)).r;
        weight += w;
    }
    return vec4<f32>(sum / max(weight, 1e-6), 0.0, 0.0, 1.0);
}

fn dx_ease(v: f32) -> f32 {
    switch p.easing {
        case 1u: { return v * v * v; }
        case 2u: { let u = 1.0 - v; return 1.0 - u * u * u; }
        case 3u: {
            if (v < 0.5) {
                return 4.0 * v * v * v;
            }
            let u = -2.0 * v + 2.0;
            return 1.0 - u * u * u * 0.5;
        }
        default: { return v; }
    }
}

// The map at uv, softened, with gamma and easing: 0 to 1, what View shows.
fn dx_map(uv: vec2<f32>) -> f32 {
    var v = 0.0;
    if (p.softness > 0.0) {
        v = lumit_read(pass_map_v, uv).r;
    } else {
        v = dx_raw(uv);
    }
    v = pow(clamp(v, 0.0, 1.0), max(p.gamma, 0.01));
    return dx_ease(v);
}

// How much of the transform the map's value v gives: Standard reads grey as
// none, Legacy black. Displacement offset adds to it everywhere.
fn dx_amount(v: f32) -> f32 {
    var a = v;
    if (p.mode == 0u) {
        a = v * 2.0 - 1.0;
    }
    return a + p.offset * 0.01;
}

// Where the edges of the picture are read from, in comp pixels.
fn dx_bounds() -> vec4<f32> {
    let f = dx_frame();
    return vec4<f32>(p.left, p.top, f.x - p.right, f.y - p.bottom);
}

fn dx_fold(x: f32, lo: f32, hi: f32) -> f32 {
    let span = max(hi - lo, 1e-3);
    let t = (x - lo) / span;
    if (p.edge == 2u) {
        return lo + fract(t) * span;
    }
    // Mirror: a triangle wave across the span.
    let m = abs(fract(t * 0.5) * 2.0 - 1.0);
    return lo + (1.0 - m) * span;
}

// The picture at q, in comp pixels, by Edge behaviour.
fn dx_read(q: vec2<f32>) -> vec4<f32> {
    let b = dx_bounds();
    var at = q;
    let outside = q.x < b.x || q.y < b.y || q.x > b.z || q.y > b.w;
    if (outside) {
        switch p.edge {
            case 1u: { at = clamp(q, b.xy, b.zw); }
            case 2u, 3u: { at = vec2<f32>(dx_fold(q.x, b.x, b.z), dx_fold(q.y, b.y, b.w)); }
            default: { return vec4<f32>(0.0); }
        }
    }
    return lumit_sample(at / dx_frame());
}

fn dx_turn(v: vec2<f32>, rad: f32) -> vec2<f32> {
    let c = cos(rad);
    let s = sin(rad);
    return vec2<f32>(v.x * c - v.y * s, v.x * s + v.y * c);
}

// The picture that lands at q when the pixel takes `a` of the transform: the
// transform turned round, read where it came from.
fn dx_take(q: vec2<f32>, a: f32) -> vec4<f32> {
    var anchor = vec2<f32>(p.anchor_x, p.anchor_y) * 0.01 * dx_frame();
    if (p.map == 2u && p.link_anchor != 0u) {
        anchor = dx_origin();
    }
    var s = vec2<f32>(p.scale_x, p.scale_y);
    if (p.uniform != 0u) {
        s = vec2<f32>(p.scale);
    }
    let k = 1.0 + (s * 0.01 - 1.0) * a * p.mult_scale;
    if (p.clamp_negative != 0u && (k.x <= 0.0 || k.y <= 0.0)) {
        return vec4<f32>(0.0);
    }
    if (abs(k.x) < 1e-4 || abs(k.y) < 1e-4) {
        return vec4<f32>(0.0);
    }
    let t = vec2<f32>(p.translate_x, p.translate_y) * a * p.mult_translate;
    let turn = radians(p.rotate) * a * p.mult_rotate;
    let back = anchor + dx_turn(q - anchor - t, -turn) / k;
    return dx_read(back);
}

// One point of the picture with every iteration and the channels' shares.
fn dx_point(q: vec2<f32>, uv: vec2<f32>) -> vec4<f32> {
    let a = dx_amount(dx_map(uv));
    let n = clamp(i32(round(p.iterations)), 1, 32);
    let spread = clamp(p.shutter * 0.01, 0.0, 1.0);
    let split = p.red != 1.0 || p.green != 1.0 || p.blue != 1.0;
    var sum = vec4<f32>(0.0);
    for (var i = 0; i < n; i += 1) {
        // From the full amount back toward none across the shutter.
        var f = 1.0;
        if (n > 1) {
            f = 1.0 - spread * f32(i) / f32(n - 1);
        }
        if (split) {
            let r = dx_take(q, a * f * p.red);
            let g = dx_take(q, a * f * p.green);
            let b = dx_take(q, a * f * p.blue);
            sum += vec4<f32>(r.r, g.g, b.b, max(r.a, max(g.a, b.a)));
        } else {
            sum += dx_take(q, a * f);
        }
    }
    return sum / f32(n);
}

fn shade(uv: vec2<f32>) -> vec4<f32> {
    if (p.view == 1u) {
        let v = dx_map(uv);
        return vec4<f32>(v, v, v, 1.0);
    }
    var grid = 1;
    if (p.antialias == 1u) {
        grid = 2;
    } else if (p.antialias == 2u) {
        grid = 4;
    }
    let frame = dx_frame();
    // One raster pixel, in comp pixels.
    let px = 1.0 / max(lumit.comp_scale, 1e-6);
    var sum = vec4<f32>(0.0);
    for (var y = 0; y < grid; y += 1) {
        for (var x = 0; x < grid; x += 1) {
            let off = (vec2<f32>(f32(x), f32(y)) + 0.5) / f32(grid) - 0.5;
            let q = uv * frame + off * px;
            sum += dx_point(q, q / frame);
        }
    }
    return sum / f32(grid * grid);
}

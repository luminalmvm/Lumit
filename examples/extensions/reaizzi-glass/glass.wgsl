// Reaizzi Glass: refractive glass for Lumit, as an LFX effect.
//
// Put it straight on a text, shape, solid, logo or precomp: the manifest's
// `backdrop` makes Lumit run it over the composite of everything below and
// hand it that layer's own picture as "Shape layer" (its alpha or luma). On an
// adjustment layer the silhouette is the layer picked in "Shape layer"; with
// none, a built-in rounded rectangle or ellipse.
//
// `@enabled_if` greys the rows that do nothing as things stand: the built-in
// shape's when a layer gives the shape, the channel when none does, Corner
// radius on an ellipse, Rim width with Edge highlight off. Lumit builds
// without it ignore the annotation and leave every row live.
//
// The glass is a height field: 0 on the edge, rising to 1 at Depth inside.
// A built-in shape has an exact distance field, so its height is analytic.
// A layer's silhouette has none, so it is blurred at quarter size and the
// blur, read as 2b - 1, stands in for the distance from the edge.
//
//   down     1/2  the shape's coverage, averaged 2x2
//   blur_h   1/4  horizontal Gaussian: r = Depth (height), g = wide (centre)
//   blur_v   1/4  vertical Gaussian of both
//   frost_h  1/2  horizontal blur of the picture below, for Frost
//   frost_v  1/2  vertical blur of it
//   shade    1    refraction, dispersion, frost, magnify, light, tint
//
// Every length is a `@unit(px)` row and is used as raster pixels, as the host
// hands it over. The host rescales those rows whenever it runs the stack on a
// raster of another size (a lowered preview while dragging, say), which it
// does not do for `lumit.comp_scale` on an adjustment layer; so nothing here
// reads comp_scale, and the glass keeps its size at every resolution.

struct Params {
    /// @choice("Shape layer", "Rounded rectangle", "Ellipse") @default("Shape layer") Shape
    shape: u32,
    /// @choice("Alpha", "Luminance", "Inverted alpha", "Inverted luminance") @default("Alpha") @enabled_if(shape = 0) @enabled_if(input) Shape channel
    channel: u32,
    /// @bounded(-50, 150) @default(50) @enabled_if(shape != 0, !input) Centre X (%)
    centre_x: f32,
    /// @bounded(-50, 150) @default(50) @enabled_if(shape != 0, !input) Centre Y (%)
    centre_y: f32,
    /// @slider(0, 4000) @default(800) @unit(px) @enabled_if(shape != 0, !input) Width
    width: f32,
    /// @slider(0, 4000) @default(450) @unit(px) @enabled_if(shape != 0, !input) Height
    height: f32,
    /// @slider(0, 1000) @default(90) @unit(px) @enabled_if(shape != 0, !input) @enabled_if(shape != 2) Corner radius
    corner: f32,
    /// @dial @default(0) @enabled_if(shape != 0, !input) Shape rotation
    rotation: f32,
    /// @slider(0, 100) @default(50) Refraction
    refraction: f32,
    /// @slider(0, 200) @default(30) @unit(px) Depth
    depth: f32,
    /// @slider(0, 100) @default(25) Dispersion
    dispersion: f32,
    /// @slider(0, 100) @default(0) @unit(px) Frost
    frost: f32,
    /// @slider(-100, 100) @default(0) Splay
    splay: f32,
    /// @slider(-100, 100) @default(0) Magnify
    magnify: f32,
    /// @bounded(0, 100) @default(100) Magnify falloff
    magnify_falloff: f32,
    /// @bounded(0.1, 4) @default(1) Effect scale
    effect_scale: f32,
    /// @toggle @default(true) Edge highlight
    edge: u32,
    /// @slider(0.5, 40) @default(3) @unit(px) @enabled_if(edge = 1) Rim width
    rim_width: f32,
    /// @dial @default(315) Light angle
    light_angle: f32,
    /// @slider(0, 200) @default(60) Light intensity
    light_intensity: f32,
    /// @default(1, 1, 1, 1) Tint
    tint: vec4<f32>,
    /// @bounded(0, 100) @default(0) Tint amount
    tint_amount: f32,
};

const RG_TAPS: i32 = 24;
const RG_RING: i32 = 12;

// ---------------------------------------------------------------------------
// Shared helpers

// The frame in raster pixels: every length below is measured in these.
fn rg_frame() -> vec2<f32> {
    return lumit_size();
}

fn rg_scale() -> f32 {
    return max(p.effect_scale, 0.01);
}

// How far the glass reaches inwards.
fn rg_depth() -> f32 {
    return max(p.depth * rg_scale(), 0.5);
}

// The wide blur that finds which way the middle of a shape is.
fn rg_wide() -> f32 {
    let f = rg_frame();
    return max(rg_depth() * 3.0, 0.12 * min(f.x, f.y));
}

fn rg_frost_radius() -> f32 {
    return p.frost * rg_scale();
}

fn rg_rim() -> f32 {
    return max(p.rim_width * rg_scale(), 0.25);
}

// A layer gives the shape only when one is picked; otherwise the built-in
// rounded rectangle stands in, so the effect always shows glass.
fn rg_uses_layer() -> bool {
    return p.shape == 0u && lumit.input2_on > 0.5;
}

fn rg_luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// The picked layer's coverage at uv, through the chosen channel.
fn rg_layer_alpha(uv: vec2<f32>) -> f32 {
    let c = lumit_sample2(uv);
    var a = c.a;
    if (p.channel == 1u || p.channel == 3u) {
        a = rg_luma(c.rgb);
    }
    a = clamp(a, 0.0, 1.0);
    if (p.channel >= 2u) {
        a = 1.0 - a;
    }
    return a;
}

fn rg_centre() -> vec2<f32> {
    return vec2<f32>(p.centre_x, p.centre_y) / 100.0 * rg_frame();
}

// Clockwise on screen (y points down), in degrees.
fn rg_rotate(v: vec2<f32>, deg: f32) -> vec2<f32> {
    let a = radians(deg);
    let c = cos(a);
    let s = sin(a);
    return vec2<f32>(c * v.x - s * v.y, s * v.x + c * v.y);
}

// Signed distance to the built-in shape, negative inside.
fn rg_sdf(px: vec2<f32>) -> f32 {
    let q = rg_rotate(px - rg_centre(), -p.rotation);
    let half = max(vec2<f32>(p.width, p.height) * 0.5, vec2<f32>(0.5, 0.5));
    if (p.shape == 2u) {
        // Ellipse: a close, cheap approximation of the true distance.
        let k0 = length(q / half);
        let k1 = length(q / (half * half));
        if (k1 < 1e-6) {
            return -min(half.x, half.y);
        }
        return k0 * (k0 - 1.0) / k1;
    }
    let r = clamp(p.corner, 0.0, min(half.x, half.y));
    let d = abs(q) - (half - vec2<f32>(r, r));
    return length(max(d, vec2<f32>(0.0, 0.0))) + min(max(d.x, d.y), 0.0) - r;
}

// One channel of a pass, Gaussian-blurred along `dir`. Sigma is half the
// radius and the taps reach three sigma.
fn rg_blur1(t: texture_2d<f32>, uv: vec2<f32>, dir: vec2<f32>, radius: f32, chan: i32) -> f32 {
    if (radius < 0.75) {
        return lumit_read(t, uv)[chan];
    }
    let sigma = radius * 0.5;
    let spacing = 3.0 * sigma / f32(RG_TAPS);
    let stride = dir * spacing / rg_frame();
    var sum = 0.0;
    var weight = 0.0;
    for (var i = -RG_TAPS; i <= RG_TAPS; i += 1) {
        let x = f32(i) * spacing;
        let w = exp(-0.5 * x * x / (sigma * sigma));
        sum += lumit_read(t, uv + stride * f32(i))[chan] * w;
        weight += w;
    }
    return sum / weight;
}

// The picture below, Gaussian-blurred along `dir`, for Frost.
fn rg_blur_below(uv: vec2<f32>, dir: vec2<f32>, radius: f32, horizontal: bool) -> vec4<f32> {
    let sigma = max(radius * 0.5, 0.5);
    let spacing = 3.0 * sigma / f32(RG_TAPS);
    let stride = dir * spacing / rg_frame();
    var sum = vec4<f32>(0.0);
    var weight = 0.0;
    for (var i = -RG_TAPS; i <= RG_TAPS; i += 1) {
        let x = f32(i) * spacing;
        let w = exp(-0.5 * x * x / (sigma * sigma));
        let at = uv + stride * f32(i);
        if (horizontal) {
            sum += lumit_sample(at) * w;
        } else {
            sum += lumit_read(pass_frost_h, at) * w;
        }
        weight += w;
    }
    return sum / weight;
}

// How much of the rim band a layer pixel sits in: 1 on the silhouette's edge,
// falling to 0 at Rim width inside. Read off the full-size silhouette, so a
// text's outline is as sharp as a built-in shape's.
fn rg_layer_band(uv: vec2<f32>, rim: f32) -> f32 {
    var inside = 0.0;
    for (var i = 0; i < RG_RING; i += 1) {
        let a = f32(i) * 6.28318531 / f32(RG_RING);
        let off = vec2<f32>(cos(a), sin(a)) * rim / rg_frame();
        inside += rg_layer_alpha(uv + off);
    }
    return clamp(2.0 * (1.0 - inside / f32(RG_RING)), 0.0, 1.0);
}

// ---------------------------------------------------------------------------
// Passes

fn down(uv: vec2<f32>) -> vec4<f32> {
    if (!rg_uses_layer()) {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(rg_layer_alpha(uv), 0.0, 0.0, 1.0);
}

fn blur_h(uv: vec2<f32>) -> vec4<f32> {
    if (!rg_uses_layer()) {
        return vec4<f32>(0.0);
    }
    let x = vec2<f32>(1.0, 0.0);
    return vec4<f32>(
        rg_blur1(pass_down, uv, x, rg_depth(), 0),
        rg_blur1(pass_down, uv, x, rg_wide(), 0),
        0.0,
        1.0
    );
}

fn blur_v(uv: vec2<f32>) -> vec4<f32> {
    if (!rg_uses_layer()) {
        return vec4<f32>(0.0);
    }
    let y = vec2<f32>(0.0, 1.0);
    return vec4<f32>(
        rg_blur1(pass_blur_h, uv, y, rg_depth(), 0),
        rg_blur1(pass_blur_h, uv, y, rg_wide(), 1),
        0.0,
        1.0
    );
}

fn frost_h(uv: vec2<f32>) -> vec4<f32> {
    if (p.frost <= 0.0) {
        return vec4<f32>(0.0);
    }
    return rg_blur_below(uv, vec2<f32>(1.0, 0.0), rg_frost_radius(), true);
}

fn frost_v(uv: vec2<f32>) -> vec4<f32> {
    if (p.frost <= 0.0) {
        return vec4<f32>(0.0);
    }
    return rg_blur_below(uv, vec2<f32>(0.0, 1.0), rg_frost_radius(), false);
}

// The picture below at a raster position, frosted as asked.
fn rg_below(px: vec2<f32>) -> vec4<f32> {
    let uv = px / rg_frame();
    if (p.frost <= 0.0) {
        return lumit_sample(uv);
    }
    return lumit_read(pass_frost_v, uv);
}

fn shade(uv: vec2<f32>) -> vec4<f32> {
    let here = lumit_load(vec2<i32>(floor(uv * rg_frame())));
    let depth = rg_depth();
    let rim = rg_rim();
    let px = uv * rg_frame();

    // Coverage, height (0 on the edge, 1 at Depth inside), the height's
    // gradient per pixel, which way the middle is, and the rim band.
    var alpha = 0.0;
    var t = 0.0;
    var grad = vec2<f32>(0.0);
    var to_mid = vec2<f32>(0.0);
    var exact_mid = false;
    var band = 0.0;
    if (rg_uses_layer()) {
        alpha = rg_layer_alpha(uv);
        if (alpha <= 0.0) {
            return here;
        }
        // One quarter-size texel either side.
        let e = 4.0 / rg_frame();
        let h = lumit_read(pass_blur_v, uv);
        let hx1 = lumit_read(pass_blur_v, uv + vec2<f32>(e.x, 0.0));
        let hx0 = lumit_read(pass_blur_v, uv - vec2<f32>(e.x, 0.0));
        let hy1 = lumit_read(pass_blur_v, uv + vec2<f32>(0.0, e.y));
        let hy0 = lumit_read(pass_blur_v, uv - vec2<f32>(0.0, e.y));
        t = clamp(2.0 * h.r - 1.0, 0.0, 1.0);
        // d(2b - 1) = 2 db, over two steps of 4 px.
        grad = vec2<f32>(hx1.r - hx0.r, hy1.r - hy0.r) / 4.0;
        to_mid = vec2<f32>(hx1.g - hx0.g, hy1.g - hy0.g);
        if (p.edge != 0u) {
            band = rg_layer_band(uv, rim);
        }
    } else {
        let d = rg_sdf(px);
        alpha = clamp(0.5 - d, 0.0, 1.0);
        if (alpha <= 0.0) {
            return here;
        }
        t = clamp(-d / depth, 0.0, 1.0);
        let k = 0.5;
        let gx = rg_sdf(px + vec2<f32>(k, 0.0)) - rg_sdf(px - vec2<f32>(k, 0.0));
        let gy = rg_sdf(px + vec2<f32>(0.0, k)) - rg_sdf(px - vec2<f32>(0.0, k));
        if (t < 1.0) {
            grad = -vec2<f32>(gx, gy) / (2.0 * k) / depth;
        }
        to_mid = rg_centre() - px;
        exact_mid = true;
        if (p.edge != 0u) {
            band = 1.0 - smoothstep(0.0, rim, max(-d, 0.0));
        }
    }

    // A rounded lens profile: steep at the rim, flat in the middle.
    let z = 1.0 - (1.0 - t) * (1.0 - t);
    let slope = grad * (2.0 * (1.0 - t)) * depth;
    var outward = vec2<f32>(0.0);
    if (length(grad) > 1e-6) {
        outward = -normalize(grad);
    }
    var mid_dir = vec2<f32>(0.0);
    if (length(to_mid) > 1e-6) {
        mid_dir = normalize(to_mid);
    }

    // Refraction reads from outside the rim inwards; Splay pushes the
    // distortion away from the middle.
    var bend = -slope * (p.refraction / 100.0) * depth * 0.75;
    bend -= mid_dir * (p.splay / 100.0) * depth * 1.5 * (1.0 - z);

    // Magnify: read towards the middle (bulge) or away from it (curve in).
    let weight = mix(1.0, z, clamp(p.magnify_falloff / 100.0, 0.0, 1.0));
    var lens = vec2<f32>(0.0);
    if (exact_mid) {
        lens = to_mid * (p.magnify / 100.0) * 0.5 * weight;
    } else {
        lens = mid_dir * (p.magnify / 100.0) * depth * 2.0 * weight;
    }

    // Dispersion: blue bends more than red, as in real glass.
    let disp = clamp(p.dispersion / 100.0, 0.0, 1.0) * 0.8;
    let green = rg_below(px + bend + lens);
    let red = rg_below(px + bend * (1.0 - disp) + lens).r;
    let blue = rg_below(px + bend * (1.0 + disp) + lens).b;
    var glass = vec4<f32>(red, green.g, blue, green.a);

    // Tint multiplies the glass by its colour.
    let ta = clamp(p.tint_amount / 100.0, 0.0, 1.0);
    glass = vec4<f32>(mix(glass.rgb, glass.rgb * p.tint.rgb, ta), glass.a);

    // Light. A soft glow near the edge always; with Edge highlight on, a lit
    // outline all round (brightest facing the light, fainter opposite, where
    // light is reflected inside the glass) and a highlight on the bevel.
    let a = radians(p.light_angle);
    let towards = vec2<f32>(sin(a), -cos(a));
    let facing = dot(outward, towards);
    let lit = pow(max(facing, 0.0), 1.5) + 0.35 * pow(max(-facing, 0.0), 1.5);
    let glow = pow(1.0 - z, 3.0) * 0.25;
    var edge_light = 0.0;
    if (p.edge != 0u) {
        let bevel = exp(-(t * depth) / rim) * lit;
        edge_light = 1.2 * bevel + band * (0.45 + 0.55 * lit);
    }
    let light = (edge_light + 0.5 * glow) * max(p.light_intensity, 0.0) / 100.0;
    glass = vec4<f32>(glass.rgb + vec3<f32>(light), max(glass.a, clamp(light, 0.0, 1.0)));

    return mix(here, glass, alpha);
}

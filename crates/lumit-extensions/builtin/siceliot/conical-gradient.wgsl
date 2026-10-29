// Conical gradient: a ramp between two circles, the Start circle swelling and
// sliding into the End circle, as Cavalry's Gradient Shader set to Conical.
// Animating the Start centre and Start radius moves a bright core through
// the shape.

struct Params {
    /// @point @default(960, 540) Start centre
    start_centre: vec2<f32>,
    /// @slider(0, 2000) @default(0) @unit(px) Start radius
    start_radius: f32,
    /// @point @default(960, 540) End centre
    end_centre: vec2<f32>,
    /// @slider(0, 4000) @default(800) @unit(px) End radius
    end_radius: f32,
    /// @choice("Pad", "Repeat", "Mirror") @default("Pad") Spread
    spread: u32,
    /// @choice("Inside layer", "Whole frame") @default("Inside layer") Paint
    paint: u32,
    /// @choice("Linear", "Smooth", "Constant") @default("Linear") Interpolation
    interpolation: u32,
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

// Where along the cone a point sits: x is t, y is 1 when some circle of the
// cone covers the point and 0 when none does.
fn cg_cone(q: vec2<f32>) -> vec2<f32> {
    let cd = p.end_centre - p.start_centre;
    let pd = q - p.start_centre;
    let r0 = p.start_radius;
    let dr = p.end_radius - r0;
    // |pd - t cd| = r0 + t dr, squared: a t^2 - 2 b t + c = 0.
    let a = dot(cd, cd) - dr * dr;
    let b = dot(pd, cd) + r0 * dr;
    let c = dot(pd, pd) - r0 * r0;
    if (abs(a) < 1e-3) {
        if (abs(b) < 1e-6) {
            return vec2<f32>(0.0, 0.0);
        }
        let t = c / (2.0 * b);
        return vec2<f32>(t, select(0.0, 1.0, r0 + t * dr >= 0.0));
    }
    let disc = b * b - a * c;
    if (disc < 0.0) {
        return vec2<f32>(0.0, 0.0);
    }
    let s = sqrt(disc);
    let hi = max((b + s) / a, (b - s) / a);
    let lo = min((b + s) / a, (b - s) / a);
    // The larger t wins, as long as its circle has a radius.
    if (r0 + hi * dr >= 0.0) {
        return vec2<f32>(hi, 1.0);
    }
    if (r0 + lo * dr >= 0.0) {
        return vec2<f32>(lo, 1.0);
    }
    return vec2<f32>(0.0, 0.0);
}

// Pad holds the end colours, Repeat starts again, Mirror runs back.
fn sc_spread(t: f32) -> f32 {
    if (p.spread == 1u) {
        return fract(t);
    }
    if (p.spread == 2u) {
        let m = t - 2.0 * floor(t * 0.5);
        return select(m, 2.0 - m, m > 1.0);
    }
    return clamp(t, 0.0, 1.0);
}

// Inside layer paints only where the layer draws, as a Cavalry fill shader.
fn sc_paint(uv: vec2<f32>, c: vec4<f32>) -> vec4<f32> {
    if (p.paint == 0u) {
        return c * lumit_sample(uv).a;
    }
    return c;
}

// ---- The SICELIOT ramp: the same text in every SICELIOT gradient ----

// The first Stops colours, sorted by Position, read at t (0 to 1).
// Colours are mixed premultiplied, so a clear stop fades without a fringe.
fn sc_ramp(t: f32) -> vec4<f32> {
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
    let hit = cg_cone(lumit_px(uv));
    let c = select(vec4<f32>(0.0), sc_ramp(sc_spread(hit.x)), hit.y > 0.5);
    return sc_paint(uv, c);
}

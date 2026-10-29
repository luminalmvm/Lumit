// Sweep gradient: the ramp turns round a centre, like a radar or a colour
// wheel. Arc limits the turn and Repetitions repeats the ramp inside it.

struct Params {
    /// @point @default(960, 540) Centre
    centre: vec2<f32>,
    /// @dial @default(0) Rotation
    rotation: f32,
    /// @slider(1, 360) @default(360) @unit(deg) Arc
    arc: f32,
    /// @counter(1, 64) @default(1) Repetitions
    repetitions: i32,
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
    let d = lumit_px(uv) - p.centre;
    var a = degrees(atan2(d.y, d.x)) - p.rotation;
    a = a - 360.0 * floor(a / 360.0);
    let t = a / max(p.arc, 1e-3) * f32(max(p.repetitions, 1));
    return sc_paint(uv, sc_ramp(sc_spread(t)));
}

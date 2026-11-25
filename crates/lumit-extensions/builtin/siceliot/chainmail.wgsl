// Chainmail: interlocking metal rings, as Octane's Chainmail. Rings sit on
// a square grid and on the grid half a step over; where two cross, one
// passes over at the first crossing and under at the second, so they link.
// Each ring is shaded round its wire, brightest along the middle.

struct Params {
    /// @slider(4, 500) @default(40) @unit(px) Ring radius
    ring_radius: f32,
    /// @slider(1, 100) @default(8) @unit(px) Ring width
    ring_width: f32,
    /// @default(0.75, 0.76, 0.78, 1) Metal colour
    metal: vec4<f32>,
    /// @default(0.05, 0.05, 0.06, 1) Background colour
    background: vec4<f32>,
    /// @bounded(0, 100) @default(60) Shading
    shading: f32,
    /// @dial @default(0) Rotation
    rotation: f32,
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

// A ring at c: how much of the wire covers q (0 to 1), and where across the
// wire q is (-1 inside edge to 1 outside edge).
fn cm_ring(q: vec2<f32>, c: vec2<f32>) -> vec2<f32> {
    let half = max(p.ring_width, 0.5) * 0.5;
    let across = (distance(q, c) - p.ring_radius) / half;
    let px = 1.0 / (half * max(lumit.comp_scale, 1e-3));
    let cover = clamp((1.0 - abs(across)) / px, 0.0, 1.0);
    return vec2<f32>(cover, clamp(across, -1.0, 1.0));
}

fn cm_lit(across: f32) -> vec4<f32> {
    let n = sqrt(max(1.0 - across * across, 0.0));
    let k = mix(1.0, 0.25 + 0.75 * n, p.shading / 100.0);
    let c = lumit_premult(p.metal);
    return vec4<f32>(c.rgb * k, c.a);
}

fn shade(uv: vec2<f32>) -> vec4<f32> {
    // Rings of one grid 2.2 radii apart, so they clear each other; each
    // links the four of the other grid on its diagonals.
    let r = max(p.ring_radius, 1.0);
    let step = r * 2.2;
    let q = sx_turn(lumit_px(uv), p.centre, p.rotation);
    let ga = floor(q / step + 0.5) * step;
    let gb = (floor(q / step) + 0.5) * step;
    var out = lumit_premult(p.background);
    // The grid-B rings first; then each A ring nearby. An A ring crosses each
    // B ring twice in the same quarter of its turn: it passes over in the first
    // half of every quarter and under in the second, so the two link.
    for (var j = -1; j <= 1; j += 1) {
        for (var i = -1; i <= 1; i += 1) {
            let o = vec2<f32>(f32(i), f32(j)) * step;
            let b = cm_ring(q, gb + o);
            out = mix(out, cm_lit(b.y), b.x);
        }
    }
    for (var j = -1; j <= 1; j += 1) {
        for (var i = -1; i <= 1; i += 1) {
            let c = ga + vec2<f32>(f32(i), f32(j)) * step;
            let a = cm_ring(q, c);
            let d = q - c;
            if (fract(atan2(d.y, d.x) / 1.5707963) < 0.5) {
                out = mix(out, cm_lit(a.y), a.x);
            } else {
                var under = 0.0;
                for (var m = -1; m <= 1; m += 1) {
                    for (var n = -1; n <= 1; n += 1) {
                        under = max(under, cm_ring(q, gb + vec2<f32>(f32(n), f32(m)) * step).x);
                    }
                }
                out = mix(out, cm_lit(a.y), a.x * (1.0 - under));
            }
        }
    }
    return sx_paint(out, uv);
}

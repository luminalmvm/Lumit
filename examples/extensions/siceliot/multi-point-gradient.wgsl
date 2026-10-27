// Multi-point gradient: up to six coloured points, each pulling the colours
// round it by its Influence. Power sets how sharply a point's colour gives
// way to its neighbours': low blends wide, high makes cells.

struct Params {
    /// @counter(2, 6) @default(4) Points
    points: i32,
    /// @slider(0.5, 8) @default(2) Power
    power: f32,
    /// @point @default(480, 270) Point 1
    point_1: vec2<f32>,
    /// @default(1, 0.35, 0.05, 1) Colour 1
    colour_1: vec4<f32>,
    /// @slider(0, 400) @default(100) Influence 1
    influence_1: f32,
    /// @point @default(1440, 270) Point 2
    point_2: vec2<f32>,
    /// @default(0.95, 0.85, 0.1, 1) Colour 2
    colour_2: vec4<f32>,
    /// @slider(0, 400) @default(100) Influence 2
    influence_2: f32,
    /// @point @default(480, 810) Point 3
    point_3: vec2<f32>,
    /// @default(0.05, 0.25, 0.9, 1) Colour 3
    colour_3: vec4<f32>,
    /// @slider(0, 400) @default(100) Influence 3
    influence_3: f32,
    /// @point @default(1440, 810) Point 4
    point_4: vec2<f32>,
    /// @default(0.6, 0.05, 0.5, 1) Colour 4
    colour_4: vec4<f32>,
    /// @slider(0, 400) @default(100) Influence 4
    influence_4: f32,
    /// @point @default(960, 540) Point 5
    point_5: vec2<f32>,
    /// @default(1, 1, 1, 1) Colour 5
    colour_5: vec4<f32>,
    /// @slider(0, 400) @default(100) Influence 5
    influence_5: f32,
    /// @point @default(960, 150) Point 6
    point_6: vec2<f32>,
    /// @default(0, 0, 0, 1) Colour 6
    colour_6: vec4<f32>,
    /// @slider(0, 400) @default(100) Influence 6
    influence_6: f32,
    /// @choice("Inside layer", "Whole frame") @default("Inside layer") Paint
    paint: u32,
};

fn shade(uv: vec2<f32>) -> vec4<f32> {
    let q = lumit_px(uv);
    let n = clamp(p.points, 2, 6);
    let pts = array<vec2<f32>, 6>(p.point_1, p.point_2, p.point_3, p.point_4, p.point_5, p.point_6);
    let cols = array<vec4<f32>, 6>(p.colour_1, p.colour_2, p.colour_3, p.colour_4, p.colour_5, p.colour_6);
    let infl = array<f32, 6>(p.influence_1, p.influence_2, p.influence_3, p.influence_4, p.influence_5, p.influence_6);
    var sum = vec4<f32>(0.0);
    var total = 0.0;
    for (var i = 0; i < n; i += 1) {
        // Distances in hundreds of pixels keep the powers inside f32's range.
        let d = max(distance(q, pts[i]) / 100.0, 1e-3);
        let w = max(infl[i], 0.0) / 100.0 / pow(d, p.power);
        sum += lumit_premult(cols[i]) * w;
        total += w;
    }
    var c = select(vec4<f32>(0.0), sum / total, total > 0.0);
    if (p.paint == 0u) {
        c *= lumit_sample(uv).a;
    }
    return c;
}

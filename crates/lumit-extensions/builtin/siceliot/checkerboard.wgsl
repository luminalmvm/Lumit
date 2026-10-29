// Checkerboard: two colours in squares, as Cavalry's Checkerboard Shader.
// Edges are smoothed over one pixel, so turning it never shimmers.

struct Params {
    /// @slider(1, 1000) @default(120) @unit(px) Cell width
    cell_width: f32,
    /// @slider(1, 1000) @default(120) @unit(px) Cell height
    cell_height: f32,
    /// @dial @default(0) Rotation
    rotation: f32,
    /// @point @default(960, 540) Centre
    centre: vec2<f32>,
    /// @default(1, 1, 1, 1) Colour 1
    colour_1: vec4<f32>,
    /// @default(0, 0, 0, 1) Colour 2
    colour_2: vec4<f32>,
    /// @choice("Inside layer", "Whole frame") @default("Inside layer") Paint
    paint: u32,
};

fn shade(uv: vec2<f32>) -> vec4<f32> {
    let d = lumit_px(uv) - p.centre;
    let a = -radians(p.rotation);
    let r = vec2<f32>(d.x * cos(a) - d.y * sin(a), d.x * sin(a) + d.y * cos(a));
    let cell = max(vec2<f32>(p.cell_width, p.cell_height), vec2<f32>(1.0));
    let q = r / cell;
    // A square wave per axis, each edge smoothed over about one pixel.
    let aa = 1.0 / (cell * max(lumit.comp_scale, 1e-3));
    let w = clamp((abs(fract(q * 0.5) - 0.5) * 2.0 - 0.5) / max(aa, vec2<f32>(1e-4)) + 0.5, vec2<f32>(0.0), vec2<f32>(1.0));
    let k = w.x + w.y - 2.0 * w.x * w.y;
    var c = mix(lumit_premult(p.colour_1), lumit_premult(p.colour_2), k);
    if (p.paint == 0u) {
        c *= lumit_sample(uv).a;
    }
    return c;
}

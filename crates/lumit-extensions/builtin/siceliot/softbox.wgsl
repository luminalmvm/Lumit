// Softbox: the face of a studio softbox, as V-Ray's Softbox texture: a lit
// panel the size of the layer, made for reflections. The panel is Colour at
// Intensity; Top, Bottom, Left and Right set how bright each side runs, a
// frame darkens the rim, and a hot spot (or a dark spot) sits on it, full
// inside Spot inner radius and gone past Spot outer radius.

struct Params {
    /// @default(1, 1, 1, 1) Colour
    colour: vec4<f32>,
    /// @slider(0, 400) @default(70) Intensity
    intensity: f32,
    /// @bounded(0, 200) @default(100) Top
    top: f32,
    /// @bounded(0, 200) @default(100) Bottom
    bottom: f32,
    /// @bounded(0, 200) @default(100) Left
    left: f32,
    /// @bounded(0, 200) @default(100) Right
    right: f32,
    /// @bounded(0, 50) @default(6) Frame width
    frame_width: f32,
    /// @bounded(0, 100) @default(60) Frame softness
    frame_softness: f32,
    /// @default(0, 0, 0, 1) Frame colour
    frame_colour: vec4<f32>,
    /// @choice("None", "Hot spot", "Dark spot") @default("Hot spot") Spot
    spot: u32,
    /// @point @default(960, 540) @enabled_if(spot=1, spot=2) Spot centre
    spot_centre: vec2<f32>,
    /// @slider(0, 4000) @default(80) @unit(px) @enabled_if(spot=1, spot=2) Spot inner radius
    spot_inner: f32,
    /// @slider(1, 4000) @default(500) @unit(px) @enabled_if(spot=1, spot=2) Spot outer radius
    spot_outer: f32,
    /// @slider(0, 400) @default(100) @enabled_if(spot=1, spot=2) Spot strength
    spot_strength: f32,
    /// @slider(0.1, 10) @default(1.5) @enabled_if(spot=1, spot=2) Spot falloff
    spot_falloff: f32,
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

fn shade(uv: vec2<f32>) -> vec4<f32> {
    let px = lumit_px(uv);
    let v = mix(p.top, p.bottom, uv.y) / 100.0;
    let h = mix(p.left, p.right, uv.x) / 100.0;
    var k = p.intensity / 100.0 * v * h;
    if (p.spot != 0u) {
        let d = distance(px, p.spot_centre);
        let span = max(p.spot_outer - p.spot_inner, 1.0);
        let s = pow(clamp(1.0 - (d - p.spot_inner) / span, 0.0, 1.0), max(p.spot_falloff, 0.1));
        let amt = s * p.spot_strength / 100.0;
        if (p.spot == 1u) {
            k += amt * p.intensity / 100.0;
        } else {
            k *= max(1.0 - amt, 0.0);
        }
    }
    let c = lumit_premult(p.colour);
    var out = vec4<f32>(c.rgb * k, c.a);
    // The frame: the rim of the panel, Frame width in from each side as a
    // share of the shorter side, eased by Frame softness.
    let fw = p.frame_width / 100.0;
    if (fw > 0.0) {
        let size = lumit_size() / max(lumit.comp_scale, 1e-6);
        let short = min(size.x, size.y);
        let edge = min(min(uv.x, 1.0 - uv.x) * size.x, min(uv.y, 1.0 - uv.y) * size.y) / short;
        let soft = max(fw * p.frame_softness / 100.0, 1e-4);
        let in_frame = 1.0 - smoothstep(fw - soft, fw, edge);
        out = mix(out, lumit_premult(p.frame_colour), in_frame);
    }
    return sx_paint(out, uv);
}

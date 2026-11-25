// Magic: swirling psychedelic colour, as Blender's Magic texture. Three
// waves, one per colour channel, are fed back into each other Depth times,
// each pass bending the next by Distortion, so the bands fold into loops and
// eyes. Evolution moves through the pattern and loops every 360 degrees.
//
// The first three waves and the closing colour follow svm_magic in Blender
// Cycles, Apache-2.0.

struct Params {
    /// @counter(0, 10) @default(2) Depth
    depth: i32,
    /// @slider(0, 10) @default(1) Distortion
    distortion: f32,
    /// @slider(4, 4000) @default(400) @unit(px) Scale
    scale: f32,
    /// @dial @default(0) Evolution
    evolution: f32,
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

fn shade(uv: vec2<f32>) -> vec4<f32> {
    let q = sx_turn(lumit_px(uv), p.centre, p.rotation) / max(p.scale, 1.0);
    // The third axis is Evolution round a circle, so a turn comes back.
    let e = radians(p.evolution);
    let z = 0.5 * sin(e);
    var x = sin((q.x + q.y + z) * 5.0);
    var y = cos((-q.x + q.y - z) * 5.0);
    var w = -cos((-q.x - q.y + z + 0.5 * cos(e)) * 5.0);
    let dist = max(p.distortion, 0.0);
    for (var i = 0; i < clamp(p.depth, 0, 10); i += 1) {
        // Each pass reads the last through the next channel round, scaled by
        // Distortion, so the channels braid.
        let k = dist * (1.0 + 0.25 * f32(i & 1));
        let nx = cos(x - y - w) * k;
        let ny = sin(-x + y - w) * k;
        let nw = cos(x + y + w) * k;
        x = select(nx, -nx, (i & 2) != 0);
        y = ny;
        w = select(nw, -nw, (i & 1) != 0);
    }
    if (dist != 0.0) {
        let d = 2.0 * dist;
        x /= d;
        y /= d;
        w /= d;
    }
    let rgb = clamp(vec3<f32>(0.5 - y, 0.5 - x, 0.5 - w), vec3<f32>(0.0), vec3<f32>(1.0));
    return sx_paint(vec4<f32>(rgb, 1.0), uv);
}

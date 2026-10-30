// Falloff: Cavalry's Falloff as a picture. It draws where an effect should
// act, from 1 (full) to 0 (none), so it can be picked as the Matte of any
// other effect, or fade its own layer.
//
// Several Falloffs on one layer combine through their Blend row, the way
// Cavalry stacks falloffs with layer modes: Multiply keeps the overlap,
// Screen or Add joins them, Subtract cuts one out of another.

struct Params {
    /// @choice("Circle", "Box", "Linear", "Sweep") @default("Circle") Shape
    shape: u32,
    /// @point @default(960, 540) Centre
    centre: vec2<f32>,
    /// @slider(0, 4000) @default(800) @unit(px) Width
    width: f32,
    /// @slider(0, 4000) @default(800) @unit(px) Height
    height: f32,
    /// @dial @default(0) Rotation
    rotation: f32,
    /// @bounded(0, 100) @default(50) Softness
    softness: f32,
    /// @slider(1, 360) @default(360) @unit(deg) Arc
    arc: f32,
    /// @counter(1, 64) @default(1) Repetitions
    repetitions: i32,
    /// @choice("Linear", "Smooth", "Ease in", "Ease out", "Bell") @default("Smooth") Curve
    curve: u32,
    /// @bounded(0, 100) @default(100) Strength
    strength: f32,
    /// @toggle @default(0) Invert falloff
    invert: u32,
    /// @toggle @default(0) Binary
    binary: u32,
    /// @slider(1, 200) @default(20) @unit(px) Cell size
    cell_size: f32,
    /// @seed Seed
    seed: u32,
    /// @choice("Greyscale", "Alpha", "Fade layer") @default("Greyscale") Output
    output: u32,
};

const PI: f32 = 3.14159265;

// Two numbers and both seeds, mixed into one, 0 to 1.
fn fo_rand(a: u32, b: u32) -> f32 {
    var h = a ^ (b * 0x9e3779b9u) ^ (p.seed * 0x85ebca6bu) ^ lumit.seed;
    h ^= h >> 16u;
    h *= 0x7feb352du;
    h ^= h >> 15u;
    h *= 0x846ca68bu;
    h ^= h >> 16u;
    return f32(h >> 8u) / 16777216.0;
}

// The point turned into the falloff's own axes.
fn fo_local(px: vec2<f32>) -> vec2<f32> {
    let d = px - p.centre;
    let a = -radians(p.rotation);
    return vec2<f32>(d.x * cos(a) - d.y * sin(a), d.x * sin(a) + d.y * cos(a));
}

// How far into the falloff a point is: 1 at its core, 0 at its edge and past
// it. Circle and Box fade over Softness; Linear and Sweep ramp across their
// whole length and repeat.
fn fo_value(px: vec2<f32>) -> f32 {
    let q = fo_local(px);
    let half = max(vec2<f32>(p.width, p.height) * 0.5, vec2<f32>(1e-3));
    let soft = max(p.softness / 100.0, 1e-4);
    let reps = f32(max(p.repetitions, 1));
    if (p.shape == 0u) {
        return clamp((1.0 - length(q / half)) / soft, 0.0, 1.0);
    }
    if (p.shape == 1u) {
        let e = abs(q) / half;
        return clamp((1.0 - max(e.x, e.y)) / soft, 0.0, 1.0);
    }
    if (p.shape == 2u) {
        let t = q.x / max(p.width, 1e-3) + 0.5;
        if (t < 0.0) {
            return 1.0;
        }
        if (t >= 1.0) {
            return 0.0;
        }
        return 1.0 - fract(t * reps);
    }
    // Sweep: round from the Rotation, clockwise, over the Arc.
    var a = degrees(atan2(q.y, q.x));
    a = a - 360.0 * floor(a / 360.0);
    let t = a / max(p.arc, 1e-3);
    if (t > 1.0) {
        return 0.0;
    }
    return 1.0 - fract(min(t, 0.99999) * reps);
}

fn fo_curve(v: f32) -> f32 {
    switch p.curve {
        case 1u: { return v * v * (3.0 - 2.0 * v); }
        case 2u: { return v * v; }
        case 3u: { return 1.0 - (1.0 - v) * (1.0 - v); }
        case 4u: { return sin(PI * v); }
        default: { return v; }
    }
}

fn shade(uv: vec2<f32>) -> vec4<f32> {
    let px = lumit_px(uv);
    var v = fo_curve(fo_value(px));
    if (p.invert == 1u) {
        v = 1.0 - v;
    }
    v = clamp(v * p.strength / 100.0, 0.0, 1.0);
    if (p.binary == 1u) {
        // Cavalry's Probability: each cell is on with a chance of the value.
        let cell = vec2<i32>(floor(px / max(p.cell_size, 1.0)));
        v = select(0.0, 1.0, fo_rand(u32(cell.x), u32(cell.y)) < v);
    }
    if (p.output == 1u) {
        return vec4<f32>(v, v, v, v);
    }
    if (p.output == 2u) {
        return lumit_sample(uv) * v;
    }
    return vec4<f32>(v, v, v, 1.0);
}

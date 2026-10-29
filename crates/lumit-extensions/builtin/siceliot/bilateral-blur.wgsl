// Bilateral blur: smooths areas of similar colour and keeps the edges
// between them sharp, as Cavalry's Bilateral Blur Filter. A neighbour counts
// less the more its colour differs, by Tolerance. Drawn across, then down.

struct Params {
    /// @slider(0, 100) @default(8) @unit(px) Radius
    radius: f32,
    /// @bounded(1, 100) @default(20) Tolerance
    tolerance: f32,
};

const BB_TAPS: i32 = 12;

fn bb_weight(i: i32, centre: vec4<f32>, s: vec4<f32>) -> f32 {
    let x = f32(i) / f32(BB_TAPS);
    let diff = length(s - centre) / max(p.tolerance / 100.0, 1e-3);
    return exp(-3.0 * x * x) * exp(-diff * diff);
}

// Pass one: across.
fn across(uv: vec2<f32>) -> vec4<f32> {
    let centre = lumit_sample(uv);
    let step = vec2<f32>(p.radius / f32(BB_TAPS) * lumit.comp_scale / lumit_size().x, 0.0);
    var sum = vec4<f32>(0.0);
    var total = 0.0;
    for (var i = -BB_TAPS; i <= BB_TAPS; i += 1) {
        let s = lumit_sample(uv + step * f32(i));
        let w = bb_weight(i, centre, s);
        sum += s * w;
        total += w;
    }
    return sum / total;
}

// Pass two: down, over pass one.
fn down(uv: vec2<f32>) -> vec4<f32> {
    let centre = lumit_read(pass_across, uv);
    let step = vec2<f32>(0.0, p.radius / f32(BB_TAPS) * lumit.comp_scale / lumit_size().y);
    var sum = vec4<f32>(0.0);
    var total = 0.0;
    for (var i = -BB_TAPS; i <= BB_TAPS; i += 1) {
        let s = lumit_read(pass_across, uv + step * f32(i));
        let w = bb_weight(i, centre, s);
        sum += s * w;
        total += w;
    }
    return sum / total;
}

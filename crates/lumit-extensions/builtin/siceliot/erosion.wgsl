// Erosion: shrinks the layer's shape (Amount above 0, a choke) or grows it
// (Amount below 0, a spread), as Cavalry's Erosion Filter. Each pixel takes
// the most transparent (or the most opaque) pixel within Amount, across then
// down, so the corners stay square. Past the layer's edges counts as clear.

struct Params {
    /// @slider(-64, 64) @default(8) @unit(px) Amount
    amount: f32,
};

const ER_MAX: i32 = 64;

fn er_load(xy: vec2<i32>) -> vec4<f32> {
    let size = vec2<i32>(lumit_size());
    if (any(xy < vec2<i32>(0)) || any(xy >= size)) {
        return vec4<f32>(0.0);
    }
    return lumit_load(xy);
}

fn er_reach() -> i32 {
    return clamp(i32(round(abs(p.amount) * lumit.comp_scale)), 0, ER_MAX);
}

// Keeps whichever of two pixels wins: the clearer one to shrink, the more
// opaque one to grow.
fn er_pick(best: vec4<f32>, s: vec4<f32>) -> vec4<f32> {
    if (p.amount >= 0.0) {
        return select(best, s, s.a < best.a);
    }
    return select(best, s, s.a > best.a);
}

// Pass one: across.
fn rows(uv: vec2<f32>) -> vec4<f32> {
    let xy = vec2<i32>(floor(uv * lumit_size()));
    let reach = er_reach();
    var best = er_load(xy);
    for (var i = -ER_MAX; i <= ER_MAX; i += 1) {
        if (abs(i) <= reach) {
            best = er_pick(best, er_load(xy + vec2<i32>(i, 0)));
        }
    }
    return best;
}

// Pass two: down, reading pass one.
fn cols(uv: vec2<f32>) -> vec4<f32> {
    let size = lumit_size();
    let xy = vec2<i32>(floor(uv * size));
    let reach = er_reach();
    var best = lumit_read(pass_rows, (vec2<f32>(xy) + 0.5) / size);
    for (var i = -ER_MAX; i <= ER_MAX; i += 1) {
        let y = xy.y + i;
        if (abs(i) <= reach && y >= 0 && y < i32(size.y)) {
            best = er_pick(best, lumit_read(pass_rows, (vec2<f32>(f32(xy.x), f32(y)) + 0.5) / size));
        } else if (abs(i) <= reach) {
            best = er_pick(best, vec4<f32>(0.0));
        }
    }
    return best;
}

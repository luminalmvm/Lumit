// Voronoi: a cellular pattern, as Cavalry's Voronoi Shader. Points are
// scattered one per cell; each pixel is measured against the nearest ones.
// Cells paints each cell one colour from the ramp, Distance shades from each
// point outwards, Edges draws the borders between cells. Evolution moves the
// points round, so it loops every 360 degrees.

struct Params {
    /// @choice("Cells", "Distance", "Edges") @default("Cells") Mode
    mode: u32,
    /// @slider(4, 1000) @default(160) @unit(px) Cell size
    cell_size: f32,
    /// @bounded(0, 100) @default(100) Randomness
    randomness: f32,
    /// @dial @default(0) Evolution
    evolution: f32,
    /// @point @default(0, 0) Offset
    offset: vec2<f32>,
    /// @slider(0, 100) @default(4) @unit(px) Edge width
    edge_width: f32,
    /// @seed Seed
    seed: u32,
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

fn vo_hash(c: vec2<i32>, salt: u32) -> f32 {
    var h = (bitcast<u32>(c.x) * 0x8da6b343u) ^ (bitcast<u32>(c.y) * 0xd8163841u) ^ (salt * 0xcb1ab31fu);
    h ^= (p.seed * 0x9e3779b9u) ^ lumit.seed;
    h ^= h >> 16u;
    h *= 0x7feb352du;
    h ^= h >> 15u;
    h *= 0x846ca68bu;
    h ^= h >> 16u;
    return f32(h >> 8u) / 16777216.0;
}

// The point of cell c, in cell units, moving round a circle as Evolution turns.
fn vo_point(c: vec2<i32>) -> vec2<f32> {
    let r = p.randomness / 100.0;
    let phase = radians(p.evolution) + 6.2831853 * vo_hash(c, 3u);
    let home = vec2<f32>(vo_hash(c, 1u), vo_hash(c, 2u));
    let wander = 0.5 + 0.35 * vec2<f32>(cos(phase), sin(phase));
    return vec2<f32>(c) + mix(vec2<f32>(0.5), mix(home, wander, 0.5), r);
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
    let size = max(p.cell_size, 1.0);
    let q = (lumit_px(uv) - p.offset) / size;
    let base = vec2<i32>(floor(q));
    var f1 = 1e9;
    var f2 = 1e9;
    var near = base;
    var near_pt = vec2<f32>(0.0);
    for (var y = -2; y <= 2; y += 1) {
        for (var x = -2; x <= 2; x += 1) {
            let c = base + vec2<i32>(x, y);
            let d = distance(q, vo_point(c));
            if (d < f1) {
                f2 = f1;
                f1 = d;
                near = c;
                near_pt = vo_point(c);
            } else if (d < f2) {
                f2 = d;
            }
        }
    }
    var t = vo_hash(near, 7u);
    if (p.mode == 1u) {
        t = clamp(f1, 0.0, 1.0);
    }
    if (p.mode == 2u) {
        // The border sits where the two nearest points are the same distance
        // away; (f2 - f1) * size is roughly twice the pixels to it.
        let to_edge = (f2 - f1) * size * 0.5;
        t = 1.0 - smoothstep(p.edge_width * 0.5, p.edge_width * 0.5 + 1.0, to_edge);
    }
    var c = sc_ramp(t);
    if (p.paint == 0u) {
        c *= lumit_sample(uv).a;
    }
    return c;
}

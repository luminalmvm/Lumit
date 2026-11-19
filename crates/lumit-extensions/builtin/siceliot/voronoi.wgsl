// Voronoi: a cellular pattern, as Cavalry's Voronoi Shader and Blender's
// Voronoi texture. Points are scattered one per cell; each pixel is measured
// against the nearest ones. Cells paints each cell one colour from the ramp,
// Distance shades from the nearest point outwards (F1), Edges draws the
// borders between cells. F2 shades from the second nearest point, Smooth F1
// blends the nearest few so the creases round off, Distance to edge is the
// true distance to the nearest border, N-sphere radius is half the gap from
// a cell's point to its nearest neighbour. Flagstone sets each cell in a line
// as stones (or leather, with a soft line), Chess cells splits each cell in
// four by turns, Plasma is F1 over F2. Evolution moves the points round,
// so it loops every 360 degrees.

struct Params {
    /// @choice("Cells", "Distance", "Edges", "F2", "Smooth F1", "Distance to edge", "N-sphere radius", "Flagstone", "Chess cells", "Plasma") @default("Cells") Mode
    mode: u32,
    /// @slider(4, 1000) @default(160) @unit(px) Cell size
    cell_size: f32,
    /// @bounded(0, 100) @default(100) Randomness
    randomness: f32,
    /// @dial @default(0) Evolution
    evolution: f32,
    /// @point @default(0, 0) Offset
    offset: vec2<f32>,
    /// @slider(0, 100) @default(4) @unit(px) @enabled_if(mode=2, mode=7) Edge width
    edge_width: f32,
    /// @default(0.08, 0.075, 0.07, 1) @enabled_if(mode=7) Line colour
    line_colour: vec4<f32>,
    /// @choice("Euclidean", "Manhattan", "Chebychev", "Minkowski") @default("Euclidean") Distance metric
    metric: u32,
    /// @slider(0.1, 10) @default(0.5) @enabled_if(metric=3) Exponent
    exponent: f32,
    /// @bounded(0, 100) @default(100) @enabled_if(mode=4, mode=7) Smoothness
    smoothness: f32,
    /// @counter(0, 8) @default(0) @enabled_if(mode=1, mode=3, mode=4) Detail
    detail: i32,
    /// @bounded(0, 100) @default(50) @enabled_if(mode=1, mode=3, mode=4) Roughness
    roughness: f32,
    /// @slider(1, 8) @default(2) @enabled_if(mode=1, mode=3, mode=4) Lacunarity
    lacunarity: f32,
    /// @seed Seed
    seed: u32,
    /// @choice("Inside layer", "Whole frame") @default("Inside layer") Paint
    paint: u32,
    /// @choice("Linear", "Smooth", "Constant") @default("Linear") Interpolation
    interpolation: u32,
    /// @choice("Linear", "Quadratic", "Easing") @default("Linear") Progression
    progression: u32,
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

// The distance by the chosen metric. Euclidean is the built-in `distance`,
// so the three modes Voronoi always had draw as they did.
fn vo_dist(a: vec2<f32>, b: vec2<f32>) -> f32 {
    let d = abs(a - b);
    switch p.metric {
        case 1u: { return d.x + d.y; }
        case 2u: { return max(d.x, d.y); }
        case 3u: {
            let e = max(p.exponent, 0.1);
            return pow(pow(d.x, e) + pow(d.y, e), 1.0 / e);
        }
        default: { return distance(a, b); }
    }
}

// One layer's F1, F2 and smooth F1 at q (cell units), and the nearest cell.
struct VoLayer {
    f1: f32,
    f2: f32,
    smooth_f1: f32,
    near: vec2<i32>,
};

fn vo_layer(q: vec2<f32>) -> VoLayer {
    let base = vec2<i32>(floor(q));
    var out = VoLayer(1e9, 1e9, 8.0, base);
    // Blender's smooth minimum: each point pulls the running value toward
    // itself by how close the two already are.
    let k = max(p.smoothness / 100.0, 1e-4) * 0.5;
    for (var y = -2; y <= 2; y += 1) {
        for (var x = -2; x <= 2; x += 1) {
            let c = base + vec2<i32>(x, y);
            let d = vo_dist(q, vo_point(c));
            if (d < out.f1) {
                out.f2 = out.f1;
                out.f1 = d;
                out.near = c;
            } else if (d < out.f2) {
                out.f2 = d;
            }
            if (p.mode == 4u) {
                let h = smoothstep(0.0, 1.0, 0.5 + 0.5 * (out.smooth_f1 - d) / k);
                out.smooth_f1 = mix(out.smooth_f1, d, h) - k * h * (1.0 - h);
            }
        }
    }
    return out;
}

// The true distance from q to the nearest border, in cell units: the
// shortest way to any bisector between the nearest point and another.
fn vo_edge(q: vec2<f32>) -> f32 {
    let base = vec2<i32>(floor(q));
    var near_pt = vec2<f32>(0.0);
    var best = 1e9;
    for (var y = -1; y <= 1; y += 1) {
        for (var x = -1; x <= 1; x += 1) {
            let pt = vo_point(base + vec2<i32>(x, y));
            let d = distance(q, pt);
            if (d < best) {
                best = d;
                near_pt = pt;
            }
        }
    }
    var edge = 1e9;
    for (var y = -2; y <= 2; y += 1) {
        for (var x = -2; x <= 2; x += 1) {
            let pt = vo_point(base + vec2<i32>(x, y));
            let gap = pt - near_pt;
            if (dot(gap, gap) > 1e-8) {
                let mid = (pt + near_pt) * 0.5;
                edge = min(edge, dot(mid - q, normalize(gap)));
            }
        }
    }
    return edge;
}

// Half the gap from the nearest point to its own nearest neighbour.
fn vo_radius(q: vec2<f32>) -> f32 {
    let near = vo_layer(q).near;
    let pt = vo_point(near);
    var gap = 1e9;
    for (var y = -2; y <= 2; y += 1) {
        for (var x = -2; x <= 2; x += 1) {
            if (x != 0 || y != 0) {
                gap = min(gap, distance(pt, vo_point(near + vec2<i32>(x, y))));
            }
        }
    }
    return gap * 0.5;
}


// ---- The SICELIOT ramp: the same text in every SICELIOT gradient ----

// The first Stops colours, sorted by Position, read at t (0 to 1), after
// Progression has bent t: Quadratic squares it, Easing eases it in and out.
// Colours are mixed premultiplied, so a clear stop fades without a fringe.
fn sc_ramp(t_in: f32) -> vec4<f32> {
    var t = t_in;
    if (p.progression != 0u) {
        t = clamp(t, 0.0, 1.0);
        if (p.progression == 1u) {
            t = t * t;
        } else {
            t = t * t * (3.0 - 2.0 * t);
        }
    }
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
    let first = vo_layer(q);
    var t = vo_hash(first.near, 7u);
    switch p.mode {
        case 1u, 3u, 4u: {
            // Distance, F2 and Smooth F1, with Detail finer layers laid over:
            // each Lacunarity times smaller and Roughness times fainter.
            var amp = 1.0;
            var freq = 1.0;
            var sum = 0.0;
            var norm = 0.0;
            let octaves = clamp(p.detail, 0, 8);
            for (var o = 0; o <= octaves; o += 1) {
                var layer = first;
                if (o > 0) {
                    // Shifted per layer, so the lattices never line up.
                    layer = vo_layer(q * freq + vec2<f32>(f32(o) * 0.7548777, f32(o) * 0.5698403));
                }
                var v = layer.f1;
                if (p.mode == 3u) {
                    // F2 runs to about 1.4 cells; scaled so most of it reads.
                    v = layer.f2 * 0.75;
                }
                if (p.mode == 4u) {
                    v = layer.smooth_f1;
                }
                sum += clamp(v, 0.0, 1.0) * amp;
                norm += amp;
                amp *= p.roughness / 100.0;
                freq *= max(p.lacunarity, 1.0);
            }
            t = sum / max(norm, 1e-6);
        }
        case 2u: {
            // The border sits where the two nearest points are the same distance
            // away; (f2 - f1) * size is roughly twice the pixels to it.
            let to_edge = (first.f2 - first.f1) * size * 0.5;
            t = 1.0 - smoothstep(p.edge_width * 0.5, p.edge_width * 0.5 + 1.0, to_edge);
        }
        case 5u: {
            t = clamp(vo_edge(q) * 2.0, 0.0, 1.0);
        }
        case 6u: {
            t = clamp(vo_radius(q) * 2.0, 0.0, 1.0);
        }
        case 8u: {
            // Chess cells: each cell split in four through its own point,
            // the quarters dark and light by turns.
            let d = q - vo_point(first.near);
            let flip = vo_hash(first.near, 11u) < 0.5;
            t = select(0.0, 1.0, (d.x * d.y > 0.0) != flip);
        }
        case 9u: {
            // Plasma: the nearest distance over the second, soft everywhere.
            t = clamp(first.f1 / max(first.f2, 1e-6), 0.0, 1.0);
        }
        default: {}
    }
    var c = sc_ramp(t);
    if (p.mode == 7u) {
        // Flagstone: each cell a stone of its own colour, bevelled toward
        // its edge, set in a line Edge width wide; Smoothness softens the
        // line into a crease, as leather.
        let edge = vo_edge(q) * size;
        let half = p.edge_width * 0.5;
        let soft = max(half * p.smoothness / 100.0, 1.0 / max(lumit.comp_scale, 1e-3));
        let k = smoothstep(half - soft * 0.5, half + soft * 0.5, edge);
        let bevel = mix(0.7, 1.0, smoothstep(half, half + size * 0.15, edge));
        c = mix(lumit_premult(p.line_colour), vec4<f32>(c.rgb * bevel, c.a), k);
    }
    if (p.paint == 0u) {
        c *= lumit_sample(uv).a;
    }
    return c;
}

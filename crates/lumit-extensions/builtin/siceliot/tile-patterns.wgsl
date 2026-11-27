// Tile patterns: tiles laid in one of ten patterns with a line between
// them, as Octane's Tile patterns and Blender's Brick texture. Each tile
// takes a colour between Tile colour 1 and Tile colour 2 at random; the line
// is Line width wide and its edge is softened by Line softness. Bricks has
// Blender's row offset and squash: every Offset frequency-th row is shifted,
// every Squash frequency-th row is narrowed. Holes leaves tiles out and Edge
// roughness frays their edges, as 3ds Max's Tiles. The brick bonds (Flemish, English,
// Chain link, Herringbone, Basket weave) are LuxCore's: bricks Tile width long
// and Row height high, headers half a brick long.

struct Params {
    /// @choice("Bricks", "Fancy tiles", "Hexagons", "Scales", "Triangles", "Flemish bond", "English bond", "Chain link", "Herringbone", "Basket weave") @default("Bricks") Type
    kind: u32,
    /// @slider(4, 1000) @default(120) @unit(px) Tile width
    tile_width: f32,
    /// @slider(4, 1000) @default(60) @unit(px) @enabled_if(kind=0, kind=5, kind=6, kind=7, kind=8, kind=9) Row height
    row_height: f32,
    /// @bounded(0, 100) @default(50) @enabled_if(kind=0) Row offset
    row_offset: f32,
    /// @counter(1, 10) @default(2) @enabled_if(kind=0) Offset frequency
    offset_frequency: i32,
    /// @slider(10, 400) @default(100) @enabled_if(kind=0) Squash
    squash: f32,
    /// @counter(1, 10) @default(2) @enabled_if(kind=0) Squash frequency
    squash_frequency: i32,
    /// @slider(0, 100) @default(4) @unit(px) Line width
    line_width: f32,
    /// @bounded(0, 100) @default(0) Line softness
    line_softness: f32,
    /// @bounded(0, 100) @default(0) Holes
    holes: f32,
    /// @bounded(0, 100) @default(0) Edge roughness
    edge_roughness: f32,
    /// @default(0.45, 0.12, 0.06, 1) Tile colour 1
    colour_1: vec4<f32>,
    /// @default(0.30, 0.07, 0.035, 1) Tile colour 2
    colour_2: vec4<f32>,
    /// @default(0.65, 0.6, 0.55, 1) Line colour
    line_colour: vec4<f32>,
    /// @dial @default(0) Rotation
    rotation: f32,
    /// @point @default(960, 540) Centre
    centre: vec2<f32>,
    /// @seed Seed
    seed: u32,
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

// A hash of a lattice cell and a salt, 0 to 1, seeded by Seed and the layer.
fn sx_hash(c: vec2<i32>, salt: u32) -> f32 {
    var h = (bitcast<u32>(c.x) * 0x8da6b343u) ^ (bitcast<u32>(c.y) * 0xd8163841u) ^ (salt * 0xcb1ab31fu);
    h ^= (p.seed * 0x9e3779b9u) ^ lumit.seed;
    h ^= h >> 16u;
    h *= 0x7feb352du;
    h ^= h >> 15u;
    h *= 0x846ca68bu;
    h ^= h >> 16u;
    return f32(h >> 8u) / 16777216.0;
}

// Gradient noise, about -1 to 1, smooth to the second derivative.
fn sx_perlin(q: vec2<f32>, salt: u32) -> f32 {
    let i = vec2<i32>(floor(q));
    let f = q - floor(q);
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    var v = array<f32, 4>(0.0, 0.0, 0.0, 0.0);
    for (var k = 0; k < 4; k += 1) {
        let o = vec2<i32>(k & 1, k >> 1);
        let a = 6.2831853 * sx_hash(i + o, salt);
        v[k] = dot(vec2<f32>(cos(a), sin(a)), f - vec2<f32>(o));
    }
    return mix(mix(v[0], v[1], u.x), mix(v[2], v[3], u.x), u.y) * 1.4142135;
}

// Octaves of gradient noise, each twice as fine and `rough` times as strong,
// normalised to about -1 to 1. Each octave is shifted so no lattice lines meet.
fn sx_fbm(q: vec2<f32>, octaves: i32, rough: f32, salt: u32) -> f32 {
    var amp = 1.0;
    var freq = 1.0;
    var sum = 0.0;
    var norm = 0.0;
    for (var o = 0; o <= clamp(octaves, 0, 8); o += 1) {
        let shift = vec2<f32>(f32(o) * 0.7548777, f32(o) * 0.5698403);
        sum += sx_perlin(q * freq + shift, salt + u32(o) * 17u) * amp;
        norm += amp;
        amp *= rough;
        freq *= 2.0;
    }
    return sum / max(norm, 1e-6);
}


// What a pattern finds at a point: its tile, and how far in from the tile's
// edge the point is, in pixels.
struct Tile {
    id: vec2<i32>,
    inset: f32,
};

fn tp_bricks(q: vec2<f32>) -> Tile {
    let h = max(p.row_height, 1.0);
    let row = i32(floor(q.y / h));
    var w = max(p.tile_width, 1.0);
    var x = q.x;
    if (row % max(p.squash_frequency, 1) == 0) {
        w *= p.squash / 100.0;
    }
    if (row % max(p.offset_frequency, 1) == 0) {
        x += w * p.row_offset / 100.0;
    }
    let col = floor(x / w);
    let fx = x - col * w;
    let fy = q.y - f32(row) * h;
    let inset = min(min(fx, w - fx), min(fy, h - fy));
    return Tile(vec2<i32>(i32(col), row), inset);
}

// Pointy-top hexagons Tile width across the flats.
fn tp_hexagons(q: vec2<f32>) -> Tile {
    let s = max(p.tile_width, 1.0);
    let r = vec2<f32>(1.0, 1.7320508) * s;
    let a = q - r * floor(q / r) - r * 0.5;
    // The second lattice, half a step over in both: mod(q - r/2, r) - r/2.
    let b = q - r * floor((q - r * 0.5) / r) - r;
    var g = a;
    var centre = q - a;
    if (dot(b, b) < dot(a, a)) {
        g = b;
        centre = q - b;
    }
    let d = abs(g);
    let edge = max(d.x, dot(d, vec2<f32>(0.5, 0.8660254)));
    return Tile(vec2<i32>(floor(centre / (s * 0.5))), s * 0.5 - edge);
}

// Equilateral triangles with sides Tile width long.
fn tp_triangles(q: vec2<f32>) -> Tile {
    let s = max(p.tile_width, 1.0);
    let h = s * 0.8660254;
    // Skewed lattice coordinates: each row starts half a side further over.
    let yy = q.y / h;
    let row = floor(yy);
    let fy = yy - row;
    let shifted = q.x / s - 0.5 * yy;
    let col = floor(shifted);
    let fx = shifted - col;
    let up = fx + fy < 1.0;
    // The three edge distances in the skewed cell, back in pixels.
    var d = min(fy, min(fx, 1.0 - fx - fy));
    if (!up) {
        d = min(1.0 - fy, min(1.0 - fx, fx + fy - 1.0));
    }
    return Tile(vec2<i32>(i32(col) * 2 + select(1, 0, up), i32(row)), d * h);
}

// Fish scales: rows of circles half a scale apart, each row half a scale
// over, so the circles just cover the floor. Each row lies over the row
// above it, as roof tiles do: a point's scale is the lowest circle holding it.
fn tp_scales(q: vec2<f32>) -> Tile {
    let s = max(p.tile_width, 1.0);
    let rh = s * 0.5;
    let row = floor(q.y / rh);
    var best = Tile(vec2<i32>(0), -1e9);
    for (var k = 2; k >= -1; k -= 1) {
        let r = row + f32(k);
        let x0 = select(0.0, 0.5 * s, (i32(r) & 1) != 0);
        let col = floor((q.x - x0) / s + 0.5);
        let c = vec2<f32>(x0 + col * s, r * rh);
        let inset = s * 0.5 - distance(q, c);
        if (inset > 0.0) {
            return Tile(vec2<i32>(i32(col), i32(r)), inset);
        }
        if (inset > best.inset) {
            best = Tile(vec2<i32>(i32(col), i32(r)), inset);
        }
    }
    return best;
}

// Square tiles, each with a quarter circle in two opposite corners turned
// at random, so the arcs join into meandering paths across the floor.
fn tp_fancy(q: vec2<f32>) -> Tile {
    let s = max(p.tile_width, 1.0);
    let id = vec2<i32>(floor(q / s));
    var f = q / s - floor(q / s);
    if (sx_hash(id, 11u) < 0.5) {
        f.x = 1.0 - f.x;
    }
    let a = abs(length(f) - 0.5);
    let b = abs(length(f - vec2<f32>(1.0)) - 0.5);
    let to_line = min(a, b) * s;
    let side = select(0, 1, length(f) > 0.5 && length(f - vec2<f32>(1.0)) > 0.5);
    return Tile(id * 2 + vec2<i32>(side, 0), to_line);
}

// A row of bricks laid in a repeating run: Flemish bond is one stretcher and
// one header, Chain link two stretchers and a header; each row slides by
// Shift so the headers sit over the stretchers below.
fn tp_run(q: vec2<f32>, whole: i32, shift: f32) -> Tile {
    let h = max(p.row_height, 1.0);
    let w = max(p.tile_width, 1.0);
    let row = floor(q.y / h);
    let period = (f32(whole) + 0.5) * w;
    let x = q.x + row * shift * w;
    let cell = floor(x / period);
    let fx = x - cell * period;
    let seg = min(floor(fx / w), f32(whole));
    let start = seg * w;
    let end = select(start + w, period, seg >= f32(whole));
    let fy = q.y - row * h;
    let inset = min(min(fx - start, end - fx), min(fy, h - fy));
    return Tile(vec2<i32>(i32(cell) * 3 + i32(seg), i32(row)), inset);
}

// English bond: a row of stretchers, then a row of headers a quarter brick over.
fn tp_english(q: vec2<f32>) -> Tile {
    let h = max(p.row_height, 1.0);
    let row = floor(q.y / h);
    let headers = (i32(row) & 1) != 0;
    let w = max(p.tile_width, 1.0) * select(1.0, 0.5, headers);
    let x = q.x + select(0.0, w * 0.5, headers);
    let col = floor(x / w);
    let fx = x - col * w;
    let fy = q.y - row * h;
    let inset = min(min(fx, w - fx), min(fy, h - fy));
    return Tile(vec2<i32>(i32(col), i32(row)), inset);
}

// Herringbone: a brick lying (Tile width by Row height) and a brick standing on
// its left end, the pair repeated one brick-height up and over, and a brick
// length over and down. The nearest few repeats are searched.
fn tp_herringbone(q: vec2<f32>) -> Tile {
    let bw = max(p.row_height, 1.0);
    let bl = max(p.tile_width, bw * 1.05);
    let ic = floor((q.x + q.y) / (2.0 * bw));
    let jc = floor((q.x - q.y) / (2.0 * bl));
    var best = Tile(vec2<i32>(0), -1e9);
    for (var dj = -1; dj <= 1; dj += 1) {
        for (var di = -2; di <= 2; di += 1) {
            let i = ic + f32(di);
            let j = jc + f32(dj);
            let r = q - vec2<f32>(i * bw + j * bl, i * bw - j * bl);
            // Lying brick: [0, bl] by [0, bw].
            let a = min(min(r.x, bl - r.x), min(r.y, bw - r.y));
            if (a > best.inset) {
                best = Tile(vec2<i32>(i32(i) * 2, i32(j)), a);
            }
            // Standing brick: [0, bw] by [bw, bw + bl].
            let b = min(min(r.x, bw - r.x), min(r.y - bw, bw + bl - r.y));
            if (b > best.inset) {
                best = Tile(vec2<i32>(i32(i) * 2 + 1, i32(j)), b);
            }
        }
    }
    return best;
}

// Basket weave: squares a brick long, each holding as many bricks side by side
// as Row height fits, lying and standing by turns like a chequerboard.
fn tp_basket(q: vec2<f32>) -> Tile {
    let s = max(p.tile_width, 1.0);
    let k = max(floor(s / max(p.row_height, 1.0)), 1.0);
    let b = floor(q / s);
    var f = q - b * s;
    let standing = ((i32(b.x) + i32(b.y)) & 1) != 0;
    if (standing) {
        f = f.yx;
    }
    let sw = s / k;
    let si = floor(f.y / sw);
    let fy = f.y - si * sw;
    let inset = min(min(f.x, s - f.x), min(fy, sw - fy));
    return Tile(vec2<i32>(i32(b.x) * 16 + i32(si), i32(b.y)), inset);
}

fn shade(uv: vec2<f32>) -> vec4<f32> {
    let q = sx_turn(lumit_px(uv), p.centre, p.rotation);
    var t: Tile;
    switch p.kind {
        case 1u: { t = tp_fancy(q); }
        case 2u: { t = tp_hexagons(q); }
        case 3u: { t = tp_scales(q); }
        case 4u: { t = tp_triangles(q); }
        case 5u: { t = tp_run(q, 1, 0.75); }
        case 6u: { t = tp_english(q); }
        case 7u: { t = tp_run(q, 2, 1.25); }
        case 8u: { t = tp_herringbone(q); }
        case 9u: { t = tp_basket(q); }
        default: { t = tp_bricks(q); }
    }
    let tile = mix(lumit_premult(p.colour_1), lumit_premult(p.colour_2), sx_hash(t.id, 3u));
    // Edge roughness frays each tile's edge with noise a few line widths
    // long; Holes leaves a share of the tiles out, showing the line colour.
    var inset = t.inset;
    if (p.edge_roughness > 0.0) {
        let grain = max(p.line_width, 2.0) * 3.0;
        inset -= abs(sx_perlin(q / grain, 17u)) * grain * p.edge_roughness / 100.0;
    }
    if (sx_hash(t.id, 19u) * 100.0 < p.holes) {
        inset = -1e9;
    }
    // The line covers the first half Line width in from each edge; Line
    // softness widens the fade from one pixel to the whole half width.
    let half = p.line_width * 0.5;
    let soft = max(half * p.line_softness / 100.0, 1.0 / max(lumit.comp_scale, 1e-3));
    let k = smoothstep(half - soft * 0.5, half + soft * 0.5, inset);
    return sx_paint(mix(lumit_premult(p.line_colour), tile, k), uv);
}

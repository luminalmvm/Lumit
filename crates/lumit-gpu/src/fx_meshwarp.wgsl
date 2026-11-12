// Mesh warp: the picture bent by a grid of points, as After Effects' Mesh Warp.
// Mirrors lumit_core::fx::effects::mesh_warp op for op (§1.6: the CPU is the
// oracle).
//
// The crossings are fractions of the frame, row by row, two to a vec4. Between
// them the picture follows a Catmull-Rom surface, and every pixel solves it
// backwards by Newton's method: from its own place first, then from the
// crossing nearest it. Outside the bent grid is transparent.

struct Params {
    rows: u32,
    cols: u32,
    steps: u32,          // Newton steps a pixel, 1..12
    count: u32,          // crossings, (rows + 1) * (cols + 1)
    mix_amt: f32,        // 0..1, blended against the unprocessed input
    matte_on: f32,       // bound by the layout, unused: the matte dissolves
    _pad0: u32,
    _pad1: u32,
    frame: vec4<f32>,    // where the frame sits on the raster: x, y, width, height
    pts: array<vec4<f32>, 512>,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;
@group(0) @binding(4) var matte: texture_2d<f32>;

const MIN_DET: f32 = 1e-12;
const MAX_RESIDUAL_PX: f32 = 1.0;
const SNAP_PX: f32 = 1e-3;

fn tap(x: i32, y: i32, size: vec2<i32>) -> vec4<f32> {
    let inside = x >= 0 && x < size.x && y >= 0 && y < size.y;
    let c = clamp(vec2<i32>(x, y), vec2<i32>(0, 0), size - vec2<i32>(1, 1));
    return select(vec4<f32>(0.0, 0.0, 0.0, 0.0), textureLoad(src, c, 0), inside);
}

fn bilinear_transparent(sx: f32, sy: f32, size: vec2<i32>) -> vec4<f32> {
    let fx = sx - 0.5;
    let fy = sy - 0.5;
    let x0 = floor(fx);
    let y0 = floor(fy);
    let tx = fx - x0;
    let ty = fy - y0;
    let x0i = i32(x0);
    let y0i = i32(y0);
    let c00 = tap(x0i, y0i, size);
    let c10 = tap(x0i + 1, y0i, size);
    let c01 = tap(x0i, y0i + 1, size);
    let c11 = tap(x0i + 1, y0i + 1, size);
    let top = c00 * (1.0 - tx) + c10 * tx;
    let bottom = c01 * (1.0 - tx) + c11 * tx;
    return top * (1.0 - ty) + bottom * ty;
}

// Crossing i, row by row.
fn at_index(i: i32) -> vec2<f32> {
    let q = p.pts[i / 2];
    return select(q.xy, q.zw, (i % 2) == 1);
}

fn at(r: i32, c: i32) -> vec2<f32> {
    return at_index(r * (i32(p.cols) + 1) + c);
}

// == mesh_warp::vertex: a line beyond each edge continued straight on.
fn vertex(r: i32, c: i32) -> vec2<f32> {
    let rows = i32(p.rows);
    let cols = i32(p.cols);
    let r0 = clamp(r, 0, rows);
    let c0 = clamp(c, 0, cols);
    var rn = r0;
    if (r < 0) { rn = 1; } else if (r > rows) { rn = rows - 1; }
    var cn = c0;
    if (c < 0) { cn = 1; } else if (c > cols) { cn = cols - 1; }
    let base = at(r0, c0);
    var out = base;
    if (r != r0) {
        out = 2.0 * out - at(rn, c0);
    }
    if (c != c0) {
        let reflected = 2.0 * base - at(r0, cn);
        out = out + reflected - base;
    }
    return out;
}

fn weights(t: f32) -> vec4<f32> {
    let t2 = t * t;
    let t3 = t2 * t;
    return vec4<f32>(
        0.5 * (-t + 2.0 * t2 - t3),
        0.5 * (2.0 - 5.0 * t2 + 3.0 * t3),
        0.5 * (t + 4.0 * t2 - 3.0 * t3),
        0.5 * (-t2 + t3));
}

fn slopes(t: f32) -> vec4<f32> {
    let t2 = t * t;
    return vec4<f32>(
        0.5 * (-1.0 + 4.0 * t - 3.0 * t2),
        0.5 * (-10.0 * t + 9.0 * t2),
        0.5 * (1.0 + 8.0 * t - 9.0 * t2),
        0.5 * (-2.0 * t + 3.0 * t2));
}

struct Surface {
    s: vec2<f32>,
    su: vec2<f32>,
    sv: vec2<f32>,
};

// == mesh_warp::surface.
fn surface(u: f32, v: f32) -> Surface {
    let cols = f32(p.cols);
    let rows = f32(p.rows);
    let gu = u * cols;
    let gv = v * rows;
    let i = clamp(i32(floor(gu)), 0, i32(p.cols) - 1);
    let j = clamp(i32(floor(gv)), 0, i32(p.rows) - 1);
    let wu = weights(gu - f32(i));
    let du = slopes(gu - f32(i));
    let wv = weights(gv - f32(j));
    let dv = slopes(gv - f32(j));
    var out: Surface;
    out.s = vec2<f32>(0.0);
    out.su = vec2<f32>(0.0);
    out.sv = vec2<f32>(0.0);
    for (var b = 0; b < 4; b = b + 1) {
        for (var a = 0; a < 4; a = a + 1) {
            let q = vertex(j - 1 + b, i - 1 + a);
            out.s = out.s + wu[a] * wv[b] * q;
            out.su = out.su + du[a] * wv[b] * q * cols;
            out.sv = out.sv + wu[a] * dv[b] * q * rows;
        }
    }
    return out;
}

fn solve(start: vec2<f32>, t: vec2<f32>) -> vec2<f32> {
    var u = start.x;
    var v = start.y;
    for (var k = 0u; k < p.steps; k = k + 1u) {
        let sf = surface(u, v);
        let f = sf.s - t;
        let det = sf.su.x * sf.sv.y - sf.su.y * sf.sv.x;
        if (abs(det) < MIN_DET) {
            break;
        }
        let inv = 1.0 / det;
        u = u - (f.x * sf.sv.y - f.y * sf.sv.x) * inv;
        v = v - (sf.su.x * f.y - sf.su.y * f.x) * inv;
    }
    return vec2<f32>(u, v);
}

fn lands(uv: vec2<f32>, t: vec2<f32>, fw: f32, fh: f32) -> bool {
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
        return false;
    }
    let s = surface(uv.x, uv.y).s;
    return max(abs((s.x - t.x) * fw), abs((s.y - t.y) * fh)) <= MAX_RESIDUAL_PX;
}

@compute @workgroup_size(8, 8)
fn mesh_warp(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let ox = p.frame.x;
    let oy = p.frame.y;
    let fw = p.frame.z;
    let fh = p.frame.w;
    let px = f32(xy.x) + 0.5;
    let py = f32(xy.y) + 0.5;
    let t = vec2<f32>((px - ox) / fw, (py - oy) / fh);
    var uv = solve(t, t);
    var found = lands(uv, t, fw, fh);
    if (!found) {
        // From the crossing nearest the pixel, for a point dragged far.
        var best = 3.4e38;
        var bi = 0;
        for (var i = 0; i < i32(p.count); i = i + 1) {
            let q = at_index(i);
            let dx = (q.x - t.x) * fw;
            let dy = (q.y - t.y) * fh;
            let d = dx * dx + dy * dy;
            if (d < best) {
                best = d;
                bi = i;
            }
        }
        let per_row = i32(p.cols) + 1;
        let start = vec2<f32>(f32(bi % per_row) / f32(p.cols), f32(bi / per_row) / f32(p.rows));
        uv = solve(start, t);
        found = lands(uv, t, fw, fh);
    }
    var v4 = vec4<f32>(0.0);
    if (found) {
        var sx = ox + uv.x * fw;
        var sy = oy + uv.y * fh;
        if (abs(sx - px) < SNAP_PX && abs(sy - py) < SNAP_PX) {
            sx = px;
            sy = py;
        }
        v4 = bilinear_transparent(sx, sy, size);
    }
    textureStore(dst, xy, o * (1.0 - p.mix_amt) + v4 * p.mix_amt);
}

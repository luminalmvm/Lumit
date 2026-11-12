// Mesh warp: the picture bent by a grid of points, as After Effects' Mesh Warp.
// Mirrors lumit_core::fx::effects::mesh_warp op for op (§1.6: the CPU is the
// oracle).
//
// The crossings are fractions of the frame, row by row, each with its row and
// column handle. Each cell is a bicubic Bezier patch, and every pixel solves it
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
    pts: array<vec4<f32>, 1536>,
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

// Float k of the crossings' list: six to a crossing, x, y, row handle x and y,
// column handle x and y.
fn fl(k: i32) -> f32 {
    let q = p.pts[k / 4];
    let m = k % 4;
    return select(select(select(q.w, q.z, m == 2), q.y, m == 1), q.x, m == 0);
}

fn pt(i: i32) -> vec2<f32> {
    return vec2<f32>(fl(i * 6), fl(i * 6 + 1));
}

fn hd(i: i32) -> vec2<f32> {
    return vec2<f32>(fl(i * 6 + 2), fl(i * 6 + 3));
}

fn vd(i: i32) -> vec2<f32> {
    return vec2<f32>(fl(i * 6 + 4), fl(i * 6 + 5));
}

// The cubic Bernstein weights at t.
fn weights(t: f32) -> vec4<f32> {
    let s = 1.0 - t;
    return vec4<f32>(s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t);
}

fn slopes(t: f32) -> vec4<f32> {
    let s = 1.0 - t;
    return vec4<f32>(-3.0 * s * s, 3.0 * s * s - 6.0 * s * t, 6.0 * s * t - 3.0 * t * t, 3.0 * t * t);
}

struct Surface {
    s: vec2<f32>,
    su: vec2<f32>,
    sv: vec2<f32>,
};

// == mesh_warp::cell_net and mesh_warp::surface.
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
    let per_row = i32(p.cols) + 1;
    let a = j * per_row + i;
    let b = a + 1;
    let c = a + per_row;
    let d = c + 1;
    let pa = pt(a); let pb = pt(b); let pc = pt(c); let pd = pt(d);
    let ha = hd(a); let hb = hd(b); let hc = hd(c); let hdd = hd(d);
    let va = vd(a); let vb = vd(b); let vc = vd(c); let vdd = vd(d);
    var net: array<vec2<f32>, 16>;
    net[0] = pa;
    net[1] = pa + ha;
    net[2] = pb - hb;
    net[3] = pb;
    net[4] = pa + va;
    net[5] = pa + ha + va;
    net[6] = pb - hb + vb;
    net[7] = pb + vb;
    net[8] = pc - vc;
    net[9] = pc + hc - vc;
    net[10] = pd - hdd - vdd;
    net[11] = pd - vdd;
    net[12] = pc;
    net[13] = pc + hc;
    net[14] = pd - hdd;
    net[15] = pd;
    var out: Surface;
    out.s = vec2<f32>(0.0);
    out.su = vec2<f32>(0.0);
    out.sv = vec2<f32>(0.0);
    for (var bb = 0; bb < 4; bb = bb + 1) {
        for (var aa = 0; aa < 4; aa = aa + 1) {
            let q = net[bb * 4 + aa];
            out.s = out.s + wu[aa] * wv[bb] * q;
            out.su = out.su + du[aa] * wv[bb] * q * cols;
            out.sv = out.sv + wu[aa] * dv[bb] * q * rows;
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
            let q = pt(i);
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

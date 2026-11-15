// Stabilise: every output pixel shows the input pixel the frame's map takes it
// to. Mirrors lumit_core::stabilise::warp op for op (the CPU is the oracle).
//
// The map was worked out host-side from the analysis and the effect's rows, so
// this is one matrix multiply, one divide and a bicubic tap a pixel. Outside
// the kept rectangle (Crop's frame), and past the input's edges, the picture
// is transparent.
//
// Fill (`stabilise_fill`) mirrors lumit_core::stabilise::fill::lay: one frame
// of the fill laid over what is under it. The host runs it once a neighbour,
// farthest first, and last for the frame itself, so the nearest frame in time
// wins wherever it reaches.
//
// A sensor read row by row is undone by `unroll` (lumit_core::stabilise::warp::
// unroll): only the frame's slide, turn and growth are read per row, and the
// four-corner fit is applied once per frame, by the map. With it off a pixel
// costs what it always did.

struct Params {
    m0: vec4<f32>,    // output pixel to input pixel, row 0 in xyz
    m1: vec4<f32>,    // row 1
    m2: vec4<f32>,    // row 2
    keep: vec4<f32>,  // the kept rectangle in output pixels: x0, y0, x1, y1
    // Fill only. The map from a pixel of the frame drawn to the same point of
    // the frame laid in this pass, row by row.
    n0: vec4<f32>,
    n1: vec4<f32>,
    n2: vec4<f32>,
    rect: vec4<f32>,  // the laid frame's part inside the trims: x0, y0, x1, y1
    fill: vec4<f32>,  // x: the feather in pixels, y: 1 when there is a picture under
    // How the frame read in this pass moved while it was read: (α, β, vx, vy)
    // for the move [[α, −β], [β, α]]·q + (vx, vy) pixels a frame.
    motion: vec4<f32>,
    // The rolling shutter: x the read time in frames (nought is no term),
    // y the axis (0 along x, 1 along y), z the frame's size along it.
    readout: vec4<f32>,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var under: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// One input pixel, transparent past the edges. The coordinate is clamped and
// the value chosen afterwards, so no load ever leaves the texture (see
// fx_cornerpin.wgsl's tap for why that matters on one Windows backend).
fn tap(x: i32, y: i32, size: vec2<i32>) -> vec4<f32> {
    let inside = x >= 0 && y >= 0 && x < size.x && y < size.y;
    let c = clamp(vec2<i32>(x, y), vec2<i32>(0, 0), size - vec2<i32>(1, 1));
    return select(vec4<f32>(0.0, 0.0, 0.0, 0.0), textureLoad(src, c, 0), inside);
}

// Catmull-Rom weights for the four taps round a sample t of the way between
// the middle two.
fn weights(t: f32) -> vec4<f32> {
    let t2 = t * t;
    let t3 = t2 * t;
    return vec4<f32>(
        0.5 * (-t3 + 2.0 * t2 - t),
        0.5 * (3.0 * t3 - 5.0 * t2 + 2.0),
        0.5 * (-3.0 * t3 + 4.0 * t2 + t),
        0.5 * (t3 - t2),
    );
}

// Sixteen explicit loads, summed in a fixed order, then held between the four
// nearest pixels so a hard edge never rings.
fn bicubic(sx: f32, sy: f32, size: vec2<i32>) -> vec4<f32> {
    let fx = sx - 0.5;
    let fy = sy - 0.5;
    let x0 = floor(fx);
    let y0 = floor(fy);
    let wx = weights(fx - x0);
    let wy = weights(fy - y0);
    let xi = i32(x0);
    let yi = i32(y0);
    var acc = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    var lo = vec4<f32>(3.0e38, 3.0e38, 3.0e38, 3.0e38);
    var hi = vec4<f32>(-3.0e38, -3.0e38, -3.0e38, -3.0e38);
    for (var j = 0; j < 4; j = j + 1) {
        var row = vec4<f32>(0.0, 0.0, 0.0, 0.0);
        for (var i = 0; i < 4; i = i + 1) {
            let c = tap(xi - 1 + i, yi - 1 + j, size);
            row = row + c * wx[i];
            if (i >= 1 && i <= 2 && j >= 1 && j <= 2) {
                lo = min(lo, c);
                hi = max(hi, c);
            }
        }
        acc = acc + row * wy[j];
    }
    return clamp(acc, lo, hi);
}

// Where the frame's sensor saw the point a global shutter would put at q: that
// far along the frame's motion, by how far from the middle of the read the
// point's row was read, refined once.
fn unroll(q: vec2<f32>) -> vec2<f32> {
    let m = p.motion;
    let v = vec2<f32>(m.x * q.x - m.y * q.y + m.z, m.y * q.x + m.x * q.y + m.w);
    let rho0 = p.readout.x * ((select(q.x, q.y, p.readout.y > 0.5) - p.readout.w) / p.readout.z - 0.5);
    let q1 = q + v * rho0;
    let rho1 = p.readout.x * ((select(q1.x, q1.y, p.readout.y > 0.5) - p.readout.w) / p.readout.z - 0.5);
    return q + v * rho1;
}

@compute @workgroup_size(8, 8)
fn stabilise(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let px = f32(xy.x) + 0.5;
    let py = f32(xy.y) + 0.5;
    var v = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    let kept = px >= p.keep.x && py >= p.keep.y && px <= p.keep.z && py <= p.keep.w;
    if (kept) {
        let u = p.m0.x * px + p.m0.y * py + p.m0.z;
        let t = p.m1.x * px + p.m1.y * py + p.m1.z;
        let d = p.m2.x * px + p.m2.y * py + p.m2.z;
        if (d > 0.0) {
            var at = vec2<f32>(u / d, t / d);
            if (p.readout.x != 0.0) {
                at = unroll(at);
            }
            v = bicubic(at.x, at.y, size);
        }
    }
    textureStore(dst, xy, v);
}

// How much of the laid frame shows at q, a point of it: all of it farther than
// the feather inside its rectangle, fading to none at the edge, none outside.
fn coverage(q: vec2<f32>) -> f32 {
    let d = min(min(q.x - p.rect.x, p.rect.z - q.x), min(q.y - p.rect.y, p.rect.w - q.y));
    if (p.fill.x > 0.0) {
        return clamp(d / p.fill.x, 0.0, 1.0);
    }
    return select(0.0, 1.0, d >= 0.0);
}

@compute @workgroup_size(8, 8)
fn stabilise_fill(@builtin(global_invocation_id) gid: vec3<u32>) {
    let out_size = vec2<i32>(textureDimensions(dst));
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= out_size.x || xy.y >= out_size.y) {
        return;
    }
    let below = clamp(xy, vec2<i32>(0, 0), vec2<i32>(textureDimensions(under)) - vec2<i32>(1, 1));
    let base = select(vec4<f32>(0.0, 0.0, 0.0, 0.0), textureLoad(under, below, 0), p.fill.y > 0.5);
    var v = base;
    let px = f32(xy.x) + 0.5;
    let py = f32(xy.y) + 0.5;
    let u = p.m0.x * px + p.m0.y * py + p.m0.z;
    let t = p.m1.x * px + p.m1.y * py + p.m1.z;
    let d = p.m2.x * px + p.m2.y * py + p.m2.z;
    if (d > 0.0) {
        let qx = u / d;
        let qy = t / d;
        let a = p.n0.x * qx + p.n0.y * qy + p.n0.z;
        let b = p.n1.x * qx + p.n1.y * qy + p.n1.z;
        let e = p.n2.x * qx + p.n2.y * qy + p.n2.z;
        if (e > 0.0) {
            var at = vec2<f32>(a / e, b / e);
            if (p.readout.x != 0.0) {
                at = unroll(at);
            }
            // By coverage, never by the picture's own alpha.
            let k = coverage(at);
            if (k > 0.0) {
                v = bicubic(at.x, at.y, size) * k + base * (1.0 - k);
            }
        }
    }
    textureStore(dst, xy, v);
}

// Vector blur: each pixel averages taps along a streak whose direction and
// length come from a map. Mirrors lumit_core::fx::cpu::vector_blur op-for-op
// (§1.6: the CPU is the oracle).
//
// The map arrives on the matte binding. With no map picked the host binds the
// layer's own picture there, and with Map softness it binds a blurred copy.
// The Angle offset's cos and sin and the tap count are host-computed.

struct Params {
    turn: vec2<f32>,       // Angle offset's cos and sin
    length: f32,           // longest streak, raster px
    falloff: f32,          // 0..1
    reach: f32,            // slope measured this far either side, px
    mix_amt: f32,          // 0..1, blended against the unprocessed input
    taps: i32,             // == cpu::vector_blur_taps
    edge: u32,             // 0 transparent, 1 repeat, 2 mirror
    from_channels: u32,    // 1 = two channels push, 0 = one channel's slope
    chan_s: u32,           // CHANNEL_OPTIONS index of the slope's channel
    chan_x: u32,           // ... of the sideways push
    chan_y: u32,           // ... of the up and down one
    constant_length: u32,  // 1 = every streak the full length
    follow: u32,           // 1 = streaks bend along the map
    direction: u32,        // 0 both ways, 1 forwards, 2 backwards
    invert: u32,           // 1 = read the map the other way round
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;
@group(0) @binding(4) var matte: texture_2d<f32>;

// == cpu::channel_of.
fn channel_of(m: vec4<f32>, which: u32) -> f32 {
    switch (which) {
        case 1u: { return m.a; }
        case 2u: { return m.r; }
        case 3u: { return m.g; }
        case 4u: { return m.b; }
        default: { return m.r * 0.2126 + m.g * 0.7152 + m.b * 0.0722; }
    }
}

// == cpu::edge_index. -1 means transparent.
fn edge_idx(i: i32, len: i32, edge: u32) -> i32 {
    if (i >= 0 && i < len) {
        return i;
    }
    if (edge == 1u) {
        return clamp(i, 0, len - 1);
    }
    if (edge == 2u) {
        var m = i;
        if (m < 0) {
            m = -m;
        } else {
            m = 2 * (len - 1) - m;
        }
        return clamp(m, 0, len - 1);
    }
    return -1;
}

// The load never goes out of bounds: the coordinate is clamped and the value
// chosen afterwards (see fx_dispmap.wgsl's tap for why).
fn tap(x: i32, y: i32, size: vec2<i32>) -> vec4<f32> {
    let xi = edge_idx(x, size.x, p.edge);
    let yi = edge_idx(y, size.y, p.edge);
    let c = clamp(vec2<i32>(xi, yi), vec2<i32>(0, 0), size - vec2<i32>(1, 1));
    return select(textureLoad(src, c, 0), vec4<f32>(0.0), xi < 0 || yi < 0);
}

// The map always repeats its edge.
fn map_tap(x: i32, y: i32, size: vec2<i32>) -> vec4<f32> {
    let c = clamp(vec2<i32>(x, y), vec2<i32>(0, 0), size - vec2<i32>(1, 1));
    return textureLoad(matte, c, 0);
}

// == cpu::bilinear_edge, same arithmetic order. `from_map` reads the map.
fn bilinear(sx: f32, sy: f32, size: vec2<i32>, from_map: bool) -> vec4<f32> {
    let fx = sx - 0.5;
    let fy = sy - 0.5;
    let x0 = floor(fx);
    let y0 = floor(fy);
    let tx = fx - x0;
    let ty = fy - y0;
    let x0i = i32(x0);
    let y0i = i32(y0);
    var c00: vec4<f32>;
    var c10: vec4<f32>;
    var c01: vec4<f32>;
    var c11: vec4<f32>;
    if (from_map) {
        c00 = map_tap(x0i, y0i, size);
        c10 = map_tap(x0i + 1, y0i, size);
        c01 = map_tap(x0i, y0i + 1, size);
        c11 = map_tap(x0i + 1, y0i + 1, size);
    } else {
        c00 = tap(x0i, y0i, size);
        c10 = tap(x0i + 1, y0i, size);
        c01 = tap(x0i, y0i + 1, size);
        c11 = tap(x0i + 1, y0i + 1, size);
    }
    let top = c00 * (1.0 - tx) + c10 * tx;
    let bottom = c01 * (1.0 - tx) + c11 * tx;
    return top * (1.0 - ty) + bottom * ty;
}

fn map_read(c: u32, sx: f32, sy: f32, size: vec2<i32>) -> f32 {
    let v = channel_of(bilinear(sx, sy, size, true), c);
    if (p.invert != 0u) {
        return 1.0 - v;
    }
    return v;
}

// == cpu::vector_at: the unit vector, turned, and its strength.
fn vector_at(x: f32, y: f32, size: vec2<i32>) -> vec3<f32> {
    var v: vec2<f32>;
    if (p.from_channels != 0u) {
        v = vec2<f32>(
            (map_read(p.chan_x, x, y, size) - 0.5) * 2.0,
            (map_read(p.chan_y, x, y, size) - 0.5) * 2.0,
        );
    } else {
        let r = p.reach;
        let gx = map_read(p.chan_s, x + r, y, size) - map_read(p.chan_s, x - r, y, size);
        let gy = map_read(p.chan_s, x, y + r, size) - map_read(p.chan_s, x, y - r, size);
        v = vec2<f32>(-gy, gx);
    }
    let mag = sqrt(v.x * v.x + v.y * v.y);
    if (mag < 1e-5) {
        return vec3<f32>(0.0);
    }
    let u = v / mag;
    var strength = min(mag, 1.0);
    if (p.constant_length != 0u) {
        strength = 1.0;
    }
    return vec3<f32>(u.x * p.turn.x - u.y * p.turn.y, u.x * p.turn.y + u.y * p.turn.x, strength);
}

@compute @workgroup_size(8, 8)
fn vector_blur(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let cx = f32(xy.x) + 0.5;
    let cy = f32(xy.y) + 0.5;
    let v0 = vector_at(cx, cy, size);
    let len = p.length * v0.z;
    let n = max(p.taps, 1);
    let nf = f32(n);
    let mid = n / 2;
    var reach_u = 1.0;
    if (p.direction == 0u) {
        reach_u = 0.5;
    }
    var acc = vec4<f32>(0.0);
    var wsum = 0.0;
    for (var walk = 0; walk < 2; walk++) {
        // (first tap, taps, stride, side), as cpu::vector_blur's walks.
        var first = 0;
        var count = n;
        var stride = 1;
        var side = 1.0;
        if (p.direction == 0u) {
            if (walk == 0) {
                first = mid;
                count = n - mid;
            } else {
                first = mid - 1;
                count = mid;
                stride = -1;
                side = -1.0;
            }
        } else if (walk == 1) {
            count = 0;
        } else if (p.direction == 1u) {
            side = -1.0;
        }
        var px = cx;
        var py = cy;
        var d = v0.xy * side;
        var done = 0.0;
        for (var j = 0; j < count; j++) {
            let t = first + j * stride;
            var u = (f32(t) + 0.5) / nf;
            if (p.direction == 0u) {
                u = abs(u - 0.5);
            }
            let a = u * len;
            px += d.x * (a - done);
            py += d.y * (a - done);
            done = a;
            let wt = 1.0 - p.falloff * u / reach_u;
            acc += bilinear(px, py, size, false) * wt;
            wsum += wt;
            if (p.follow != 0u) {
                let nv = vector_at(px, py, size);
                if (nv.x != 0.0 || nv.y != 0.0) {
                    var f = 1.0;
                    if (nv.x * d.x + nv.y * d.y < 0.0) {
                        f = -1.0;
                    }
                    d = nv.xy * f;
                }
            }
        }
    }
    let o = textureLoad(src, xy, 0);
    textureStore(dst, xy, o * (1.0 - p.mix_amt) + (acc / wsum) * p.mix_amt);
}
